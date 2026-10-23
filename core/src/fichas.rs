//! FICHAS ANONIMAS do video pelo relay (patch 0006): Privacy Pass com VOPRF.
//!
//! Cada conta tem uma quota de video pelo relay por mes, configuravel no servidor. O relay das
//! chamadas tem de o fazer cumprir SEM saber quem esta na sala. Por isso:
//!
//!   * EMISSAO (relay das mensagens, com a conta autenticada): a app pede as fichas do mes
//!     num lote, CEGAS (VOPRF, RFC 9497, ristretto255-SHA512; emissao em lote no estilo do
//!     RFC 9578). O servidor so ve elementos aleatorios e conta quantas ja deu a essa conta
//!     nesse mes. Devolve-as avaliadas com uma prova DLEQ em lote; a app verifica a prova com
//!     a chave publica do mes e tira a cegueira. Fica com `(nonce, saida)`, que o servidor
//!     nunca viu.
//!   * RESGATE (relay das chamadas, ligacao anonima da sala): quem ENVIA video pelo relay
//!     entrega uma ficha no inicio e outra a cada bloco (`BLOCK_SECS`). O relay calcula a
//!     saida do VOPRF para o `nonce` com a chave do mes e compara. Nada do que recebe tem a
//!     ver com a conta: so o `nonce` (escolhido pela app) e a saida.
//!   * GASTO DUPLO: o relay guarda um resumo do `nonce` das fichas gastas (Redis, por mes).
//!   * CHAVES: uma por mes (epoca `AAAAMM`, UTC), derivadas de uma semente do servidor com o
//!     `DeriveKeyPair` do RFC 9497. Fichas de outro mes nao valem.
//!
//! A criptografia e toda da crate `voprf` (Facebook, mantida); aqui so ha formatos e
//! verificacoes de tamanho. Os pacotes do relay das chamadas (resgate, resposta, chave do
//! mes) seguem o estilo de `crate::call`: cabecalho com a sala e etiqueta HMAC da
//! `relay_key` no fim; o resgate vai dentro de um Noise_N para a chave do relay (quem esta
//! na rede nao ve a ficha).

use rand_core::{CryptoRng, OsRng, RngCore};
use sha2::{Digest, Sha256};
use thiserror::Error;
use voprf::{BlindedElement, EvaluationElement, Group, Proof, Ristretto255, VoprfClient, VoprfServer};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::call::{relay_tag, relay_tag_valid, RelayVideo, Role, RoomKeys, RELAY_TAG_LEN, ROOM_ID_LEN};

type Suite = Ristretto255;

/// Bytes do valor aleatorio de cada ficha (escolhido pela app, nunca visto na emissao).
pub const NONCE_LEN: usize = 32;
/// Saida do VOPRF (SHA-512).
pub const OUTPUT_LEN: usize = 64;
/// Um elemento do grupo (cego ou avaliado) e a chave publica: 32 bytes (ristretto255).
pub const ELEMENT_LEN: usize = 32;
pub const PUBLIC_KEY_LEN: usize = 32;
/// Prova DLEQ (dois escalares).
pub const PROOF_LEN: usize = 64;
/// Um bloco de video pelo relay por ficha: 1 minuto (ver docs/protocol-spec.md, seccao 19).
pub const BLOCK_SECS: u64 = 60;
/// Fichas por conta e por mes, por omissao (quota de video pelo relay configuravel no servidor).
pub const DEFAULT_PER_MONTH: u32 = 300;
/// Maximo de fichas num pedido de emissao (cabe num frame Noise em JSON com base64).
pub const MAX_BATCH: usize = 400;
/// Uma ficha guardada na app: `[4 epoca][32 nonce][64 saida][32 chave publica]`.
pub const FICHA_LEN: usize = 4 + NONCE_LEN + OUTPUT_LEN + PUBLIC_KEY_LEN;

/// Domínio da chave de cada mes (`DeriveKeyPair(semente, info)`; info = isto + AAAAMM).
const KEY_INFO: &[u8] = b"QT-FICHAS-v1 chave do mes ";
/// Entrada do VOPRF: isto + epoca + nonce (a ficha so vale no mes para que foi pedida).
const INPUT_PREFIX: &[u8] = b"QT-FICHA-v1";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FichaError {
    #[error("tamanho invalido")]
    Length,
    #[error("elemento do grupo invalido")]
    Element,
    #[error("prova DLEQ invalida (chave errada ou resposta adulterada)")]
    Proof,
    #[error("semente da chave invalida")]
    Key,
}

// ------------------------------------------------------------------ epocas

/// Epoca (AAAAMM, UTC) de um instante em segundos desde 1970.
pub fn epoch_of_unix(secs: u64) -> u32 {
    // Data civil a partir dos dias (algoritmo de Howard Hinnant).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y * 100 + m) as u32
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Epoca de agora (relogio deste aparelho).
pub fn current_epoch() -> u32 {
    epoch_of_unix(unix_now())
}

/// Epoca valida (mes de 1 a 12, ano razoavel).
pub fn epoch_valid(e: u32) -> bool {
    (1970..=9999).contains(&(e / 100)) && (1..=12).contains(&(e % 100))
}

/// A epoca seguinte (dezembro passa a janeiro do ano seguinte).
pub fn next_epoch(e: u32) -> u32 {
    if e % 100 >= 12 {
        (e / 100 + 1) * 100 + 1
    } else {
        e + 1
    }
}

/// Folga do relogio na viragem do mes (UTC): so ate aqui um telemovel aceita o mes vizinho como
/// o "de agora" (relogio adiantado ou atrasado). Fora disto, so o mes do proprio relogio.
pub const EPOCH_SKEW_SECS: u64 = 48 * 3600;

/// O mes `e` pode ser o de agora para um relogio em `now`? O proprio mes, ou o vizinho a menos de
/// `EPOCH_SKEW_SECS` da viragem. Um relay que dissesse a cada conta um mes diferente (o site
/// publica 24: daria para separar as contas em grupos pelas fichas que gastam, como na particao
/// de chaves) nao passa daqui.
pub fn epoch_plausible_at(e: u32, now: u64) -> bool {
    let local = epoch_of_unix(now);
    if e == local {
        return true;
    }
    if !epoch_valid(e) {
        return false;
    }
    if e == next_epoch(local) {
        return epoch_end_unix(local).saturating_sub(now) <= EPOCH_SKEW_SECS;
    }
    if next_epoch(e) == local {
        return now.saturating_sub(epoch_end_unix(e)) <= EPOCH_SKEW_SECS;
    }
    false
}

/// `epoch_plausible_at` com o relogio deste aparelho.
pub fn epoch_plausible(e: u32) -> bool {
    epoch_plausible_at(e, unix_now())
}

/// Primeiro segundo da epoca seguinte (para os prazos do Redis).
pub fn epoch_end_unix(e: u32) -> u64 {
    let (mut y, mut m) = ((e / 100) as i64, (e % 100) as i64 + 1);
    if m > 12 {
        m = 1;
        y += 1;
    }
    // Dias desde 1970 do dia 1 de (y, m) (inverso do algoritmo de cima).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    (days.max(0) as u64) * 86_400
}

fn input(epoch: u32, nonce: &[u8; NONCE_LEN]) -> [u8; INPUT_PREFIX.len() + 4 + NONCE_LEN] {
    let mut i = [0u8; INPUT_PREFIX.len() + 4 + NONCE_LEN];
    i[..INPUT_PREFIX.len()].copy_from_slice(INPUT_PREFIX);
    i[INPUT_PREFIX.len()..INPUT_PREFIX.len() + 4].copy_from_slice(&epoch.to_be_bytes());
    i[INPUT_PREFIX.len() + 4..].copy_from_slice(nonce);
    i
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Resumo do `nonce` para o registo das fichas gastas (o relay nunca guarda a ficha).
pub fn spent_id(nonce: &[u8; NONCE_LEN]) -> [u8; 16] {
    let h = Sha256::new().chain_update(b"QT-FICHA-gasta-v1").chain_update(nonce).finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&h[..16]);
    out
}

/// Identificador curto da ficha na resposta do relay (para a app saber a que pedido responde).
pub fn ack_id(nonce: &[u8; NONCE_LEN]) -> [u8; 4] {
    let h = Sha256::new().chain_update(b"QT-FICHA-resposta-v1").chain_update(nonce).finalize();
    [h[0], h[1], h[2], h[3]]
}

// ------------------------------------------------------------------ a ficha

/// Uma ficha pronta a gastar (guardada na base cifrada da app).
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct Ficha {
    pub epoch: u32,
    pub nonce: [u8; NONCE_LEN],
    pub output: [u8; OUTPUT_LEN],
    /// Chave publica do mes com que a prova da emissao foi verificada.
    pub issuer_key: [u8; PUBLIC_KEY_LEN],
}

impl std::fmt::Debug for Ficha {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Nunca escrever o nonce nem a saida (seriam uma ficha valida num registo).
        write!(f, "Ficha {{ epoca: {} }}", self.epoch)
    }
}

impl Ficha {
    pub fn to_bytes(&self) -> [u8; FICHA_LEN] {
        let mut b = [0u8; FICHA_LEN];
        b[..4].copy_from_slice(&self.epoch.to_be_bytes());
        b[4..36].copy_from_slice(&self.nonce);
        b[36..100].copy_from_slice(&self.output);
        b[100..].copy_from_slice(&self.issuer_key);
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() != FICHA_LEN {
            return None;
        }
        Some(Ficha {
            epoch: u32::from_be_bytes(b[..4].try_into().ok()?),
            nonce: b[4..36].try_into().ok()?,
            output: b[36..100].try_into().ok()?,
            issuer_key: b[100..].try_into().ok()?,
        })
    }
}

// ------------------------------------------------------------------ servidor

/// Chave do emissor de UM mes (no relay das mensagens, que emite, e no das chamadas, que
/// verifica). Derivada da semente: os dois processos chegam a mesma chave sem a trocarem.
pub struct Issuer {
    server: VoprfServer<Suite>,
    epoch: u32,
    public_key: [u8; PUBLIC_KEY_LEN],
}

impl Issuer {
    pub fn derive(seed: &[u8; 32], epoch: u32) -> Result<Self, FichaError> {
        let mut info = KEY_INFO.to_vec();
        info.extend_from_slice(format!("{epoch:06}").as_bytes());
        let server = VoprfServer::<Suite>::new_from_seed(seed, &info).map_err(|_| FichaError::Key)?;
        let mut public_key = [0u8; PUBLIC_KEY_LEN];
        public_key.copy_from_slice(&<Suite as Group>::serialize_elem(server.get_public_key()));
        Ok(Issuer { server, epoch, public_key })
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn public_key(&self) -> [u8; PUBLIC_KEY_LEN] {
        self.public_key
    }

    /// Avalia `blinded` (n x 32 bytes) e prova em lote. Devolve (n x 32 bytes avaliados, prova).
    pub fn issue(&self, blinded: &[u8]) -> Result<(Vec<u8>, [u8; PROOF_LEN]), FichaError> {
        if blinded.is_empty() || !blinded.len().is_multiple_of(ELEMENT_LEN) || blinded.len() / ELEMENT_LEN > MAX_BATCH {
            return Err(FichaError::Length);
        }
        let elems = blinded
            .chunks(ELEMENT_LEN)
            .map(BlindedElement::<Suite>::deserialize)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| FichaError::Element)?;
        let r = self.server.batch_blind_evaluate(&mut OsRng, &elems).map_err(|_| FichaError::Element)?;
        let mut out = Vec::with_capacity(blinded.len());
        for m in &r.messages {
            out.extend_from_slice(&m.serialize());
        }
        let mut proof = [0u8; PROOF_LEN];
        proof.copy_from_slice(&r.proof.serialize());
        Ok((out, proof))
    }

    /// A ficha e deste mes e foi emitida com esta chave? Tempo constante na comparacao.
    pub fn verify(&self, epoch: u32, nonce: &[u8; NONCE_LEN], output: &[u8; OUTPUT_LEN]) -> bool {
        if epoch != self.epoch {
            return false;
        }
        match self.server.evaluate(&input(epoch, nonce)) {
            Ok(o) => ct_eq(&o, output),
            Err(_) => false,
        }
    }
}

// ------------------------------------------------------------------ app

/// Pedido de emissao a meio: os fatores de cegueira ficam so em memoria ate a resposta.
pub struct IssueRequest {
    epoch: u32,
    clients: Vec<VoprfClient<Suite>>,
    nonces: Vec<[u8; NONCE_LEN]>,
}

impl Drop for IssueRequest {
    fn drop(&mut self) {
        self.nonces.zeroize();
    }
}

impl IssueRequest {
    /// `n` fichas cegas para a epoca `epoch`. Devolve o pedido (guardar) e os bytes a enviar.
    pub fn new(epoch: u32, n: usize) -> Result<(Self, Vec<u8>), FichaError> {
        Self::new_with_rng(epoch, n, &mut OsRng)
    }

    pub fn new_with_rng<R: RngCore + CryptoRng>(epoch: u32, n: usize, rng: &mut R) -> Result<(Self, Vec<u8>), FichaError> {
        if n == 0 || n > MAX_BATCH {
            return Err(FichaError::Length);
        }
        let mut clients = Vec::with_capacity(n);
        let mut nonces = Vec::with_capacity(n);
        let mut blinded = Vec::with_capacity(n * ELEMENT_LEN);
        for _ in 0..n {
            let mut nonce = [0u8; NONCE_LEN];
            rng.fill_bytes(&mut nonce);
            let b = VoprfClient::<Suite>::blind(&input(epoch, &nonce), rng).map_err(|_| FichaError::Element)?;
            blinded.extend_from_slice(&b.message.serialize());
            clients.push(b.state);
            nonces.push(nonce);
        }
        Ok((IssueRequest { epoch, clients, nonces }, blinded))
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn len(&self) -> usize {
        self.clients.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    /// Resposta do emissor: as primeiras `evaluated.len() / 32` fichas pedidas (o servidor pode
    /// dar menos, ate ao teto do mes). A prova DLEQ e verificada com `public_key`; sem ela
    /// valida, nenhuma ficha fica.
    pub fn finalize(&self, public_key: &[u8], evaluated: &[u8], proof: &[u8]) -> Result<Vec<Ficha>, FichaError> {
        if evaluated.is_empty() {
            // Nada dado (teto do mes atingido): a resposta nao traz prova.
            return Ok(Vec::new());
        }
        if public_key.len() != PUBLIC_KEY_LEN || proof.len() != PROOF_LEN || !evaluated.len().is_multiple_of(ELEMENT_LEN) {
            return Err(FichaError::Length);
        }
        let k = evaluated.len() / ELEMENT_LEN;
        if k > self.clients.len() {
            return Err(FichaError::Length);
        }
        let pk = <Suite as Group>::deserialize_elem(public_key).map_err(|_| FichaError::Element)?;
        let proof = Proof::<Suite>::deserialize(proof).map_err(|_| FichaError::Proof)?;
        let messages = evaluated
            .chunks(ELEMENT_LEN)
            .map(EvaluationElement::<Suite>::deserialize)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| FichaError::Element)?;
        let inputs: Vec<_> = self.nonces[..k].iter().map(|n| input(self.epoch, n)).collect();
        // Copia dos primeiros k (a crate quer uma colecao com tamanho conhecido); apagada no fim.
        let clients: Vec<VoprfClient<Suite>> = self.clients[..k].to_vec();
        let outputs = VoprfClient::<Suite>::batch_finalize(&inputs, &clients, &messages, &proof, pk).map_err(|_| FichaError::Proof)?;
        let issuer_key: [u8; PUBLIC_KEY_LEN] = public_key.try_into().map_err(|_| FichaError::Length)?;
        let mut out = Vec::with_capacity(k);
        for (o, nonce) in outputs.zip(&self.nonces[..k]) {
            let o = o.map_err(|_| FichaError::Proof)?;
            let output: [u8; OUTPUT_LEN] = o.as_slice().try_into().map_err(|_| FichaError::Length)?;
            out.push(Ficha { epoch: self.epoch, nonce: *nonce, output, issuer_key });
        }
        Ok(out)
    }
}

// ------------------------------------------------------------------ pacotes da sala

/// Resgate de uma ficha: `[1 0x25][Noise_N para a chave estatica do relay]`.
pub const REDEEM_KIND: u8 = 0x25;
/// Resposta do relay ao resgate.
pub const REDEEM_ACK_KIND: u8 = 0x26;
/// "Qual e a chave deste mes?" (com a etiqueta da sala) e a resposta.
pub const KEY_REQ_KIND: u8 = 0x27;
pub const KEY_RESP_KIND: u8 = 0x28;
/// Prologo do Noise_N do resgate (separa este uso da chave do relay).
pub const REDEEM_PROLOGUE: &[u8] = b"QT-CALL-FICHA-v1";
/// `[1 versao=1][1 papel][8 room_id][32 relay_key][4 epoca][32 nonce][64 saida]`.
pub const REDEEM_PAYLOAD_LEN: usize = 1 + 1 + ROOM_ID_LEN + 32 + 4 + NONCE_LEN + OUTPUT_LEN;
/// `[1 0x26][8 room_id][1 papel][4 id][1 estado][1 modo][2 segundos BE][8 etiqueta]`.
pub const REDEEM_ACK_LEN: usize = 1 + ROOM_ID_LEN + 1 + 4 + 1 + 1 + 2 + RELAY_TAG_LEN;
/// `[1 0x27][8 room_id][47 zeros][8 etiqueta]` (maior do que a resposta: nada de amplificar).
pub const KEY_REQ_LEN: usize = 64;
/// `[1 0x28][8 room_id][4 epoca][1 estado][32 chave publica][8 etiqueta]`.
pub const KEY_RESP_LEN: usize = 1 + ROOM_ID_LEN + 4 + 1 + PUBLIC_KEY_LEN + RELAY_TAG_LEN;
// O relay nunca responde com mais bytes do que recebeu (nada de amplificar ataques): o resgate
// leva 1 + 48 (Noise_N) + o conteudo.
const _: () = assert!(REDEEM_ACK_LEN < 1 + 48 + REDEEM_PAYLOAD_LEN && KEY_RESP_LEN < KEY_REQ_LEN);

/// O que o relay fez com a ficha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedeemStatus {
    /// Aceite: o lado ganhou mais um bloco de video.
    Accepted = 0,
    /// A mesma ficha de novo, na mesma sala (resposta perdida): ja tinha contado.
    Repeated = 1,
    /// Ja gasta (noutra sala ou noutra altura). Usar outra.
    Spent = 2,
    /// Nao e valida: forjada, de outro mes ou de outra chave. Usar outra.
    Invalid = 3,
    /// O relay nao consegue verificar agora (sem chave ou sem registo): a ficha NAO foi gasta.
    Unavailable = 4,
}

impl RedeemStatus {
    fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            0 => Self::Accepted,
            1 => Self::Repeated,
            2 => Self::Spent,
            3 => Self::Invalid,
            4 => Self::Unavailable,
            _ => return None,
        })
    }
}

/// Como o relay trata o video desta sala quanto a fichas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    /// O video pelo relay gasta fichas; `public_key` e a chave do mes.
    Required = 0,
    /// Este relay nao pede fichas (video livre, como antes do 0006).
    Free = 1,
    /// Pede fichas mas agora nao as consegue verificar.
    Unavailable = 2,
}

/// Resposta do relay a ficha (lida pela app).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedeemAck {
    pub role: Role,
    pub id: [u8; 4],
    pub status: RedeemStatus,
    pub mode: RelayVideo,
    /// Segundos de video que este lado tem agora no relay.
    pub credit_secs: u16,
}

pub fn redeem_payload(role: Role, room: &RoomKeys, f: &Ficha) -> [u8; REDEEM_PAYLOAD_LEN] {
    let mut p = [0u8; REDEEM_PAYLOAD_LEN];
    p[0] = 1;
    p[1] = role as u8;
    p[2..10].copy_from_slice(&room.room_id);
    p[10..42].copy_from_slice(&room.relay_key);
    p[42..46].copy_from_slice(&f.epoch.to_be_bytes());
    p[46..78].copy_from_slice(&f.nonce);
    p[78..].copy_from_slice(&f.output);
    p
}

/// Conteudo do resgate (ja decifrado pelo relay).
pub struct RedeemRequest {
    pub role: Role,
    pub room: RoomKeys,
    pub epoch: u32,
    pub nonce: [u8; NONCE_LEN],
    pub output: [u8; OUTPUT_LEN],
}

pub fn parse_redeem_payload(p: &[u8]) -> Option<RedeemRequest> {
    if p.len() != REDEEM_PAYLOAD_LEN || p[0] != 1 {
        return None;
    }
    Some(RedeemRequest {
        role: Role::from_bit(p[1])?,
        room: RoomKeys { room_id: p[2..10].try_into().ok()?, relay_key: p[10..42].try_into().ok()? },
        epoch: u32::from_be_bytes(p[42..46].try_into().ok()?),
        nonce: p[46..78].try_into().ok()?,
        output: p[78..].try_into().ok()?,
    })
}

fn mode_byte(m: RelayVideo) -> u8 {
    m as u8
}

fn mode_from(b: u8) -> Option<RelayVideo> {
    Some(match b {
        0 => RelayVideo::Full,
        1 => RelayVideo::Cap240,
        2 => RelayVideo::Denied,
        _ => return None,
    })
}

/// Resposta do relay ao resgate.
pub fn redeem_ack(room: &RoomKeys, ack: &RedeemAck) -> [u8; REDEEM_ACK_LEN] {
    let mut p = [0u8; REDEEM_ACK_LEN];
    p[0] = REDEEM_ACK_KIND;
    p[1..9].copy_from_slice(&room.room_id);
    p[9] = ack.role as u8;
    p[10..14].copy_from_slice(&ack.id);
    p[14] = ack.status as u8;
    p[15] = mode_byte(ack.mode);
    p[16..18].copy_from_slice(&ack.credit_secs.to_be_bytes());
    let tag = relay_tag(&room.relay_key, &p[..REDEEM_ACK_LEN - RELAY_TAG_LEN]);
    p[REDEEM_ACK_LEN - RELAY_TAG_LEN..].copy_from_slice(&tag);
    p
}

pub fn parse_redeem_ack(room: &RoomKeys, p: &[u8]) -> Option<RedeemAck> {
    if p.len() != REDEEM_ACK_LEN || p[0] != REDEEM_ACK_KIND || p[1..9] != room.room_id || !relay_tag_valid(&room.relay_key, p) {
        return None;
    }
    Some(RedeemAck {
        role: Role::from_bit(p[9])?,
        id: p[10..14].try_into().ok()?,
        status: RedeemStatus::from_byte(p[14])?,
        mode: mode_from(p[15])?,
        credit_secs: u16::from_be_bytes([p[16], p[17]]),
    })
}

pub fn key_request(room: &RoomKeys) -> [u8; KEY_REQ_LEN] {
    let mut p = [0u8; KEY_REQ_LEN];
    p[0] = KEY_REQ_KIND;
    p[1..9].copy_from_slice(&room.room_id);
    let tag = relay_tag(&room.relay_key, &p[..KEY_REQ_LEN - RELAY_TAG_LEN]);
    p[KEY_REQ_LEN - RELAY_TAG_LEN..].copy_from_slice(&tag);
    p
}

/// O relay le a sala do pedido (a etiqueta verifica-se com a chave da sala).
pub fn key_request_room(p: &[u8]) -> Option<[u8; ROOM_ID_LEN]> {
    if p.len() != KEY_REQ_LEN || p[0] != KEY_REQ_KIND || p[9..KEY_REQ_LEN - RELAY_TAG_LEN].iter().any(|x| *x != 0) {
        return None;
    }
    p[1..9].try_into().ok()
}

pub fn key_response(room: &RoomKeys, epoch: u32, status: KeyStatus, public_key: &[u8; PUBLIC_KEY_LEN]) -> [u8; KEY_RESP_LEN] {
    let mut p = [0u8; KEY_RESP_LEN];
    p[0] = KEY_RESP_KIND;
    p[1..9].copy_from_slice(&room.room_id);
    p[9..13].copy_from_slice(&epoch.to_be_bytes());
    p[13] = status as u8;
    p[14..46].copy_from_slice(public_key);
    let tag = relay_tag(&room.relay_key, &p[..KEY_RESP_LEN - RELAY_TAG_LEN]);
    p[KEY_RESP_LEN - RELAY_TAG_LEN..].copy_from_slice(&tag);
    p
}

/// A app le a resposta: (epoca do relay, estado, chave publica do mes).
pub fn parse_key_response(room: &RoomKeys, p: &[u8]) -> Option<(u32, KeyStatus, [u8; PUBLIC_KEY_LEN])> {
    if p.len() != KEY_RESP_LEN || p[0] != KEY_RESP_KIND || p[1..9] != room.room_id || !relay_tag_valid(&room.relay_key, p) {
        return None;
    }
    let status = match p[13] {
        0 => KeyStatus::Required,
        1 => KeyStatus::Free,
        2 => KeyStatus::Unavailable,
        _ => return None,
    };
    Some((u32::from_be_bytes(p[9..13].try_into().ok()?), status, p[14..46].try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed() -> [u8; 32] {
        [7u8; 32]
    }

    fn issue(n: usize, epoch: u32) -> (Issuer, Vec<Ficha>) {
        let iss = Issuer::derive(&seed(), epoch).unwrap();
        let (req, blinded) = IssueRequest::new(epoch, n).unwrap();
        assert_eq!(blinded.len(), n * ELEMENT_LEN);
        let (ev, proof) = iss.issue(&blinded).unwrap();
        let f = req.finalize(&iss.public_key(), &ev, &proof).unwrap();
        (iss, f)
    }

    #[test]
    fn epocas_em_utc() {
        assert_eq!(epoch_of_unix(1_792_713_600), 202610);
        assert_eq!(epoch_of_unix(1_798_761_599), 202612);
        assert_eq!(epoch_of_unix(1_798_761_600), 202701);
        assert_eq!(epoch_end_unix(202612), 1_798_761_600);
        assert_eq!(epoch_end_unix(202402), 1_709_251_200, "fim de fevereiro bissexto");
        for e in [197001, 202001, 202402, 202610, 202612, 209912] {
            assert_eq!(epoch_of_unix(epoch_end_unix(e) - 1), e);
            assert_ne!(epoch_of_unix(epoch_end_unix(e)), e);
        }
        assert!(epoch_valid(202610) && !epoch_valid(202613) && !epoch_valid(202600) && !epoch_valid(12));
        assert_eq!((next_epoch(202610), next_epoch(202612)), (202611, 202701));
    }

    #[test]
    fn mes_do_relay_so_o_proprio_ou_o_vizinho_na_viragem() {
        let viragem = epoch_end_unix(202610); // 1 de novembro, 00:00 UTC
        let meio = viragem - 15 * 86_400;
        assert!(epoch_plausible_at(202610, meio));
        assert!(!epoch_plausible_at(202611, meio), "a meio do mes, o seguinte nao");
        assert!(!epoch_plausible_at(202609, meio), "nem o anterior");
        assert!(!epoch_plausible_at(209901, meio) && !epoch_plausible_at(202613, meio), "nem um mes qualquer");
        assert!(epoch_plausible_at(202611, viragem - 3600), "uma hora antes da viragem");
        assert!(epoch_plausible_at(202610, viragem + 3600), "uma hora depois, o anterior ainda");
        assert!(!epoch_plausible_at(202611, viragem - EPOCH_SKEW_SECS - 1));
        assert!(!epoch_plausible_at(202610, viragem + EPOCH_SKEW_SECS + 1));
        assert!(!epoch_plausible_at(202612, viragem - 3600), "so o vizinho");
        let ano = epoch_end_unix(202612);
        assert!(epoch_plausible_at(202701, ano - 60) && epoch_plausible_at(202612, ano + 60), "dezembro e janeiro");
    }

    #[test]
    fn emitir_e_verificar() {
        let (iss, f) = issue(5, 202610);
        assert_eq!(f.len(), 5);
        for x in &f {
            assert!(iss.verify(x.epoch, &x.nonce, &x.output));
            assert_eq!(x.issuer_key, iss.public_key());
            assert_eq!(Ficha::from_bytes(&x.to_bytes()).as_ref(), Some(x));
        }
        // Todas diferentes (nonces aleatorios).
        assert_ne!(f[0].nonce, f[1].nonce);
        // A mesma chave sai sempre da mesma semente e do mesmo mes.
        assert_eq!(Issuer::derive(&seed(), 202610).unwrap().public_key(), iss.public_key());
    }

    #[test]
    fn forjada_de_outro_mes_ou_de_outra_chave_recusada() {
        let (iss, f) = issue(1, 202610);
        let x = &f[0];
        let mut out = x.output;
        out[0] ^= 1;
        assert!(!iss.verify(x.epoch, &x.nonce, &out), "saida adulterada");
        let mut n = x.nonce;
        n[5] ^= 1;
        assert!(!iss.verify(x.epoch, &n, &x.output), "nonce trocado");
        assert!(!iss.verify(202611, &x.nonce, &x.output), "outro mes");
        let next = Issuer::derive(&seed(), 202611).unwrap();
        assert_ne!(next.public_key(), iss.public_key(), "rotacao: chave nova por mes");
        assert!(!next.verify(202611, &x.nonce, &x.output));
        assert!(!Issuer::derive(&[8u8; 32], 202610).unwrap().verify(x.epoch, &x.nonce, &x.output), "outra semente");
        assert!(!iss.verify(x.epoch, &[0u8; 32], &[0u8; 64]), "inventada");
    }

    #[test]
    fn prova_dleq_obrigatoria() {
        let iss = Issuer::derive(&seed(), 202610).unwrap();
        let (req, blinded) = IssueRequest::new(202610, 3).unwrap();
        let (ev, proof) = iss.issue(&blinded).unwrap();
        // Chave errada (o servidor a tentar marcar quem pediu com uma chave so dele): recusa.
        let other = Issuer::derive(&[9u8; 32], 202610).unwrap();
        assert_eq!(req.finalize(&other.public_key(), &ev, &proof), Err(FichaError::Proof));
        // Avaliado com outra chave mas apresentado com a publica: recusa.
        let (ev2, proof2) = other.issue(&blinded).unwrap();
        assert_eq!(req.finalize(&iss.public_key(), &ev2, &proof2), Err(FichaError::Proof));
        let mut bad = proof;
        bad[3] ^= 1;
        assert!(req.finalize(&iss.public_key(), &ev, &bad).is_err());
        // Menos do que o pedido (teto do mes): so as primeiras, com prova para essas.
        let (ev3, proof3) = iss.issue(&blinded[..2 * ELEMENT_LEN]).unwrap();
        let f = req.finalize(&iss.public_key(), &ev3, &proof3).unwrap();
        assert_eq!(f.len(), 2);
        assert!(f.iter().all(|x| iss.verify(x.epoch, &x.nonce, &x.output)));
        // Mais do que o pedido, tamanhos errados, elementos invalidos.
        assert!(req.finalize(&iss.public_key(), &[ev.clone(), ev.clone()].concat(), &proof).is_err());
        assert!(iss.issue(&[0u8; 31]).is_err());
        assert!(iss.issue(&[0u8; 32]).is_err(), "identidade");
        assert!(iss.issue(&vec![0u8; (MAX_BATCH + 1) * 32]).is_err());
        assert!(IssueRequest::new(202610, 0).is_err() && IssueRequest::new(202610, MAX_BATCH + 1).is_err());
    }

    #[test]
    fn o_emissor_so_ve_elementos_aleatorios() {
        // O que o servidor recebe na emissao nao contem o nonce nem a saida, e dois pedidos
        // do mesmo nonce dao elementos cegos diferentes.
        let mut rng = OsRng;
        let nonce = [5u8; 32];
        let a = VoprfClient::<Suite>::blind(&input(202610, &nonce), &mut rng).unwrap().message.serialize();
        let b = VoprfClient::<Suite>::blind(&input(202610, &nonce), &mut rng).unwrap().message.serialize();
        assert_ne!(a, b);
        let iss = Issuer::derive(&seed(), 202610).unwrap();
        let (req, blinded) = IssueRequest::new(202610, 4).unwrap();
        let (ev, proof) = iss.issue(&blinded).unwrap();
        let f = req.finalize(&iss.public_key(), &ev, &proof).unwrap();
        // Nem o pedido nem a resposta tem pedacos do nonce ou da saida de nenhuma ficha.
        for x in &f {
            for part in x.nonce.chunks(8).chain(x.output.chunks(8)) {
                assert!(!blinded.windows(8).any(|w| w == part) && !ev.windows(8).any(|w| w == part));
            }
        }
    }

    #[test]
    fn pacotes_da_sala() {
        let room = RoomKeys::derive(&[1; 32], &[3; 16]);
        let (_, f) = issue(1, 202610);
        let p = redeem_payload(Role::Callee, &room, &f[0]);
        let r = parse_redeem_payload(&p).unwrap();
        assert_eq!((r.role, r.room.room_id, r.room.relay_key, r.epoch, r.nonce, r.output), (Role::Callee, room.room_id, room.relay_key, 202610, f[0].nonce, f[0].output));
        assert!(parse_redeem_payload(&p[1..]).is_none());

        let ack = RedeemAck { role: Role::Callee, id: ack_id(&f[0].nonce), status: RedeemStatus::Accepted, mode: RelayVideo::Cap240, credit_secs: 60 };
        let a = redeem_ack(&room, &ack);
        assert_eq!(parse_redeem_ack(&room, &a), Some(ack));
        let mut t = a;
        t[16] ^= 1;
        assert_eq!(parse_redeem_ack(&room, &t), None, "adulterada");
        assert_eq!(parse_redeem_ack(&RoomKeys::derive(&[2; 32], &[3; 16]), &a), None, "outra sala");

        let q = key_request(&room);
        assert_eq!(key_request_room(&q), Some(room.room_id));
        assert!(relay_tag_valid(&room.relay_key, &q));
        let resp = key_response(&room, 202610, KeyStatus::Required, &[4u8; 32]);
        assert!(resp.len() < q.len(), "resposta menor que o pedido");
        assert_eq!(parse_key_response(&room, &resp), Some((202610, KeyStatus::Required, [4u8; 32])));
        let mut t = resp;
        t[20] ^= 1;
        assert_eq!(parse_key_response(&room, &t), None);
        let mut bad = q;
        bad[20] = 1;
        assert!(key_request_room(&bad).is_none());
        // Ninguem confunde estes pacotes com audio ou video.
        assert!(crate::call::parse_header(&q).is_none() && !crate::call::is_video(&q));
    }

    #[test]
    fn debug_nao_mostra_a_ficha() {
        let (_, f) = issue(1, 202610);
        assert_eq!(format!("{:?}", f[0]), "Ficha { epoca: 202610 }");
    }
}
