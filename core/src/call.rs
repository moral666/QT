//! Chamadas de voz: chaves da chamada e pacotes de audio (estilo WireGuard).
//!
//! O "aperto de mao" da chamada NAO acontece aqui: a oferta e a resposta vao
//! dentro do Double Ratchet (ver `qt_ffi::call`), que ja autentica os dois lados
//! e e pos-quantico (PQXDH). Cada lado manda 32 bytes aleatorios (`offer_secret`
//! e `answer_secret`); daqui sai tudo o que a chamada precisa, por HKDF-SHA256:
//!
//!   * SO da oferta (os dois lados ja os tem enquanto o telemovel toca, para entrar
//!     na sala antes de atender, o "pre-aquecimento"):
//!     - `room_id` (8 bytes): o codigo da sala no relay. Nao identifica contas.
//!     - `relay_key` (32 bytes): so autentica pacotes perante o relay (etiqueta de
//!       8 bytes no fim), para ele aceitar a mudanca de IP (Wi-Fi para 4G) sem que
//!       outra pessoa consiga desviar a sala. NAO decifra nada.
//!   * Da oferta E da resposta: uma chave ChaCha20-Poly1305 por sentido (quem liga ->
//!     quem atende, e o contrario). Um pacote refletido de volta nunca e aceite.
//!
//! PACOTE DE AUDIO (tamanho FIXO durante a chamada, igual para todos):
//!   [1 tipo = 0x10 | papel][8 room_id][8 contador BE]
//!   [ChaCha20-Poly1305( [2 tamanho BE][frame][zeros ate `pad`] ) + 16 tag]
//!   [8 etiqueta do relay = HMAC-SHA256(relay_key, tudo o que vem antes)]
//! O cabecalho (17 bytes) e dado adicional do AEAD. Nonce = [4 zeros][8 contador LE],
//! como no WireGuard; o contador nunca se repete com a mesma chave. A rececao tem
//! uma janela anti-repeticao de 1024 pacotes (aceita pacotes fora de ordem, recusa
//! repetidos e muito antigos). Frames vazios (tamanho 0) mantem a sala aberta.
//!
//! ENTRAR NA SALA (relay): `[1 0x20][Noise_N para a chave estatica do relay]` com o
//! conteudo `[1 versao=1][1 papel][8 room_id][32 relay_key]` (ver `join_payload`). O
//! relay fica a saber a sala e a chave das etiquetas, nunca a conta. Responde
//! `[1 0x21][8 room_id][1 o outro ja esta na sala][8 etiqueta]` (`join_ok`), mais
//! pequeno que o pedido (nao serve para amplificar ataques).

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Key, Nonce,
};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const CALL_ID_LEN: usize = 16;
pub const SECRET_LEN: usize = 32;
pub const ROOM_ID_LEN: usize = 8;
pub const RELAY_TAG_LEN: usize = 8;
/// Tipo do pacote de audio; o bit 0 e o papel de quem envia (0 liga, 1 atende).
pub const MEDIA_KIND: u8 = 0x10;
/// Pedido para entrar na sala (cifrado para o relay) e a resposta do relay.
pub const JOIN_KIND: u8 = 0x20;
pub const JOIN_OK_KIND: u8 = 0x21;
pub const JOIN_PAYLOAD_LEN: usize = 1 + 1 + ROOM_ID_LEN + 32;
pub const JOIN_OK_LEN: usize = 1 + ROOM_ID_LEN + 1 + RELAY_TAG_LEN;
/// Prologo do Noise_N do pedido de entrada (separa este uso da chave do relay).
pub const JOIN_PROLOGUE: &[u8] = b"QT-CALL-JOIN-v1";
pub const HEADER_LEN: usize = 1 + ROOM_ID_LEN + 8;
const AEAD_TAG_LEN: usize = 16;
/// Bytes que cada pacote tem a mais que o `pad` (cabecalho, tag do AEAD, etiqueta do relay).
pub const MEDIA_OVERHEAD: usize = HEADER_LEN + AEAD_TAG_LEN + RELAY_TAG_LEN;
/// Limites do tamanho cifrado de cada frame (inclui os 2 bytes do tamanho).
pub const MIN_PAD: usize = 32;
pub const MAX_PAD: usize = 1024;
/// Janela anti-repeticao (pacotes).
pub const REPLAY_WINDOW: u64 = 1024;
/// O contador nunca passa disto (a 50 pacotes/s seriam milhares de anos).
pub const MAX_COUNTER: u64 = 1 << 48;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CallError {
    #[error("frame demasiado grande para o tamanho fixo da chamada")]
    FrameTooLarge,
    #[error("tamanho fixo invalido")]
    BadPad,
    #[error("pacote com tamanho ou cabecalho errado")]
    Malformed,
    #[error("pacote de outra sala ou do sentido errado")]
    WrongRoom,
    #[error("pacote repetido ou demasiado antigo")]
    Replay,
    #[error("falha ao decifrar (pacote adulterado ou chave errada)")]
    Decrypt,
    #[error("contador esgotado: terminar a chamada")]
    Exhausted,
}

/// Quem envia: quem liga (`Caller`) ou quem atende (`Callee`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Caller = 0,
    Callee = 1,
}

impl Role {
    pub fn other(self) -> Role {
        match self {
            Role::Caller => Role::Callee,
            Role::Callee => Role::Caller,
        }
    }
    fn bit(self) -> u8 {
        self as u8
    }
    pub fn from_bit(b: u8) -> Option<Role> {
        match b {
            0 => Some(Role::Caller),
            1 => Some(Role::Callee),
            _ => None,
        }
    }
}

/// Codigo da sala e chave das etiquetas: so dependem da oferta.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct RoomKeys {
    pub room_id: [u8; ROOM_ID_LEN],
    pub relay_key: [u8; 32],
}

impl RoomKeys {
    pub fn derive(offer_secret: &[u8; SECRET_LEN], call_id: &[u8; CALL_ID_LEN]) -> Self {
        let hk = Hkdf::<Sha256>::new(Some(call_id), offer_secret);
        let mut room_id = [0u8; ROOM_ID_LEN];
        let mut relay_key = [0u8; 32];
        hk.expand(b"QT-CALL-v1 room", &mut room_id).expect("hkdf room");
        hk.expand(b"QT-CALL-v1 relay", &mut relay_key).expect("hkdf relay");
        RoomKeys { room_id, relay_key }
    }
}

/// Chaves de uma chamada (apagadas da memoria quando saem de uso).
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct CallKeys {
    room_id: [u8; ROOM_ID_LEN],
    relay_key: [u8; 32],
    send_key: [u8; 32],
    recv_key: [u8; 32],
    #[zeroize(skip)]
    role: Role,
}

impl CallKeys {
    /// Deriva as chaves a partir dos dois segredos trocados dentro do ratchet.
    /// Os dois lados obtem o mesmo `room_id` e `relay_key` (os de `RoomKeys`), e
    /// chaves de envio e rececao trocadas entre si.
    pub fn derive(offer_secret: &[u8; SECRET_LEN], answer_secret: &[u8; SECRET_LEN], call_id: &[u8; CALL_ID_LEN], role: Role) -> Self {
        let mut ikm = [0u8; 2 * SECRET_LEN];
        ikm[..SECRET_LEN].copy_from_slice(offer_secret);
        ikm[SECRET_LEN..].copy_from_slice(answer_secret);
        let hk = Hkdf::<Sha256>::new(Some(call_id), &ikm);
        ikm.zeroize();
        let room = RoomKeys::derive(offer_secret, call_id);
        let (room_id, relay_key) = (room.room_id, room.relay_key);
        let mut to_callee = [0u8; 32];
        let mut to_caller = [0u8; 32];
        // Os tamanhos pedidos sao sempre validos para HKDF-SHA256 (<= 255 x 32).
        hk.expand(b"QT-CALL-v1 caller->callee", &mut to_callee).expect("hkdf send");
        hk.expand(b"QT-CALL-v1 callee->caller", &mut to_caller).expect("hkdf recv");
        let (send_key, recv_key) = match role {
            Role::Caller => (to_callee, to_caller),
            Role::Callee => (to_caller, to_callee),
        };
        to_callee.zeroize();
        to_caller.zeroize();
        CallKeys { room_id, relay_key, send_key, recv_key, role }
    }

    pub fn room_id(&self) -> [u8; ROOM_ID_LEN] {
        self.room_id
    }

    /// Chave que o relay recebe (na fase do relay) para validar as etiquetas.
    pub fn relay_key(&self) -> [u8; 32] {
        self.relay_key
    }

    pub fn role(&self) -> Role {
        self.role
    }
}

/// Tamanho total de cada pacote de uma chamada com este `pad`.
pub fn packet_len(pad: usize) -> usize {
    MEDIA_OVERHEAD + pad
}

fn nonce(counter: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&counter.to_le_bytes());
    n
}

fn relay_tag(relay_key: &[u8; 32], data: &[u8]) -> [u8; RELAY_TAG_LEN] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(relay_key).expect("hmac aceita qualquer chave");
    mac.update(data);
    let full = mac.finalize().into_bytes();
    let mut tag = [0u8; RELAY_TAG_LEN];
    tag.copy_from_slice(&full[..RELAY_TAG_LEN]);
    tag
}

/// Cabecalho de um pacote de audio, lido sem chaves (o que o relay ve).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaHeader {
    pub role: Role,
    pub room_id: [u8; ROOM_ID_LEN],
    pub counter: u64,
}

/// Le o cabecalho (para o relay encaminhar). Nao verifica nada criptografico.
pub fn parse_header(packet: &[u8]) -> Option<MediaHeader> {
    if packet.len() < MEDIA_OVERHEAD + MIN_PAD || packet.len() > MEDIA_OVERHEAD + MAX_PAD {
        return None;
    }
    let role = match packet[0] {
        k if k == MEDIA_KIND => Role::Caller,
        k if k == MEDIA_KIND | 1 => Role::Callee,
        _ => return None,
    };
    let mut room_id = [0u8; ROOM_ID_LEN];
    room_id.copy_from_slice(&packet[1..1 + ROOM_ID_LEN]);
    let counter = u64::from_be_bytes(packet[1 + ROOM_ID_LEN..HEADER_LEN].try_into().ok()?);
    Some(MediaHeader { role, room_id, counter })
}

/// O relay confirma que o pacote veio de alguem da chamada (para aceitar um IP novo).
/// Comparacao em tempo constante.
pub fn relay_tag_valid(relay_key: &[u8; 32], packet: &[u8]) -> bool {
    if packet.len() <= RELAY_TAG_LEN {
        return false;
    }
    let (data, tag) = packet.split_at(packet.len() - RELAY_TAG_LEN);
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(relay_key).expect("hmac aceita qualquer chave");
    mac.update(data);
    mac.verify_truncated_left(tag).is_ok()
}

/// Conteudo do pedido para entrar na sala (vai cifrado para o relay com Noise_N).
pub fn join_payload(role: Role, room: &RoomKeys) -> [u8; JOIN_PAYLOAD_LEN] {
    let mut p = [0u8; JOIN_PAYLOAD_LEN];
    p[0] = 1;
    p[1] = role.bit();
    p[2..2 + ROOM_ID_LEN].copy_from_slice(&room.room_id);
    p[2 + ROOM_ID_LEN..].copy_from_slice(&room.relay_key);
    p
}

/// O relay le o pedido (ja decifrado).
pub fn parse_join_payload(p: &[u8]) -> Option<(Role, RoomKeys)> {
    if p.len() != JOIN_PAYLOAD_LEN || p[0] != 1 {
        return None;
    }
    let role = Role::from_bit(p[1])?;
    let mut room_id = [0u8; ROOM_ID_LEN];
    let mut relay_key = [0u8; 32];
    room_id.copy_from_slice(&p[2..2 + ROOM_ID_LEN]);
    relay_key.copy_from_slice(&p[2 + ROOM_ID_LEN..]);
    Some((role, RoomKeys { room_id, relay_key }))
}

/// Resposta do relay ao pedido de entrada.
pub fn join_ok(room: &RoomKeys, peer_present: bool) -> [u8; JOIN_OK_LEN] {
    let mut p = [0u8; JOIN_OK_LEN];
    p[0] = JOIN_OK_KIND;
    p[1..1 + ROOM_ID_LEN].copy_from_slice(&room.room_id);
    p[1 + ROOM_ID_LEN] = peer_present as u8;
    let tag = relay_tag(&room.relay_key, &p[..JOIN_OK_LEN - RELAY_TAG_LEN]);
    p[JOIN_OK_LEN - RELAY_TAG_LEN..].copy_from_slice(&tag);
    p
}

/// O telemovel confirma a resposta do relay. `Some(o outro ja esta na sala)`.
pub fn parse_join_ok(room: &RoomKeys, p: &[u8]) -> Option<bool> {
    if p.len() != JOIN_OK_LEN || p[0] != JOIN_OK_KIND || p[1..1 + ROOM_ID_LEN] != room.room_id {
        return None;
    }
    if !relay_tag_valid(&room.relay_key, p) || p[1 + ROOM_ID_LEN] > 1 {
        return None;
    }
    Some(p[1 + ROOM_ID_LEN] == 1)
}

/// Janela anti-repeticao (RFC 6479, como no WireGuard): `top` e o proximo contador
/// "novo"; os bits guardam quais dos ultimos `REPLAY_WINDOW` ja chegaram.
#[derive(Clone)]
struct ReplayWindow {
    top: u64,
    bits: [u64; (REPLAY_WINDOW / 64) as usize],
}

impl ReplayWindow {
    fn new() -> Self {
        ReplayWindow { top: 0, bits: [0; (REPLAY_WINDOW / 64) as usize] }
    }
    fn slot(n: u64) -> (usize, u64) {
        let i = n % REPLAY_WINDOW;
        ((i / 64) as usize, 1u64 << (i % 64))
    }
    fn check(&self, n: u64) -> bool {
        if n >= self.top {
            return true;
        }
        if self.top - n > REPLAY_WINDOW {
            return false;
        }
        let (w, b) = Self::slot(n);
        self.bits[w] & b == 0
    }
    fn update(&mut self, n: u64) {
        if n >= self.top {
            if n + 1 - self.top >= REPLAY_WINDOW {
                self.bits = [0; (REPLAY_WINDOW / 64) as usize];
            } else {
                for i in self.top..=n {
                    let (w, b) = Self::slot(i);
                    self.bits[w] &= !b;
                }
            }
            self.top = n + 1;
        }
        let (w, b) = Self::slot(n);
        self.bits[w] |= b;
    }
}

/// Cifra e decifra os pacotes de audio de UMA chamada.
pub struct MediaSession {
    keys: CallKeys,
    pad: usize,
    send_counter: u64,
    replay: ReplayWindow,
}

impl MediaSession {
    /// `pad` = tamanho fixo do frame cifrado (2 bytes de tamanho + audio + zeros).
    pub fn new(keys: CallKeys, pad: usize) -> Result<Self, CallError> {
        if !(MIN_PAD..=MAX_PAD).contains(&pad) {
            return Err(CallError::BadPad);
        }
        Ok(MediaSession { keys, pad, send_counter: 0, replay: ReplayWindow::new() })
    }

    pub fn room_id(&self) -> [u8; ROOM_ID_LEN] {
        self.keys.room_id
    }

    pub fn role(&self) -> Role {
        self.keys.role
    }

    pub fn relay_key(&self) -> [u8; 32] {
        self.keys.relay_key
    }

    /// Maior frame de audio que cabe num pacote.
    pub fn max_frame(&self) -> usize {
        self.pad - 2
    }

    pub fn packet_len(&self) -> usize {
        packet_len(self.pad)
    }

    /// Cifra um frame (pode ser vazio). Todos os pacotes tem o mesmo tamanho.
    pub fn seal(&mut self, frame: &[u8]) -> Result<Vec<u8>, CallError> {
        if frame.len() > self.max_frame() {
            return Err(CallError::FrameTooLarge);
        }
        if self.send_counter >= MAX_COUNTER {
            return Err(CallError::Exhausted);
        }
        let counter = self.send_counter;
        self.send_counter += 1;

        let mut out = Vec::with_capacity(self.packet_len());
        out.push(MEDIA_KIND | self.keys.role.bit());
        out.extend_from_slice(&self.keys.room_id);
        out.extend_from_slice(&counter.to_be_bytes());

        let mut plain = vec![0u8; self.pad];
        plain[..2].copy_from_slice(&(frame.len() as u16).to_be_bytes());
        plain[2..2 + frame.len()].copy_from_slice(frame);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.keys.send_key));
        let n = nonce(counter);
        let ct = cipher
            .encrypt(Nonce::from_slice(&n), Payload { msg: &plain, aad: &out })
            .map_err(|_| CallError::Decrypt)?;
        plain.zeroize();
        out.extend_from_slice(&ct);
        let tag = relay_tag(&self.keys.relay_key, &out);
        out.extend_from_slice(&tag);
        Ok(out)
    }

    /// Decifra um pacote do outro lado. Devolve (contador, frame); o contador serve
    /// para ordenar no jitter buffer. Pacotes invalidos devem ser descartados em silencio.
    pub fn open(&mut self, packet: &[u8]) -> Result<(u64, Vec<u8>), CallError> {
        if packet.len() != self.packet_len() {
            return Err(CallError::Malformed);
        }
        let h = parse_header(packet).ok_or(CallError::Malformed)?;
        if h.role != self.keys.role.other() || h.room_id != self.keys.room_id {
            return Err(CallError::WrongRoom);
        }
        if h.counter >= MAX_COUNTER {
            return Err(CallError::Malformed);
        }
        // Verificacao barata antes de decifrar; a janela so avanca depois do AEAD passar.
        if !self.replay.check(h.counter) {
            return Err(CallError::Replay);
        }
        let aad = &packet[..HEADER_LEN];
        let ct = &packet[HEADER_LEN..packet.len() - RELAY_TAG_LEN];
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.keys.recv_key));
        let n = nonce(h.counter);
        let mut plain = cipher.decrypt(Nonce::from_slice(&n), Payload { msg: ct, aad }).map_err(|_| CallError::Decrypt)?;
        self.replay.update(h.counter);
        let len = u16::from_be_bytes([plain[0], plain[1]]) as usize;
        if len > self.pad - 2 {
            plain.zeroize();
            return Err(CallError::Malformed);
        }
        let frame = plain[2..2 + len].to_vec();
        plain.zeroize();
        Ok((h.counter, frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(pad: usize) -> (MediaSession, MediaSession) {
        let (o, a, id) = ([1u8; 32], [2u8; 32], [3u8; 16]);
        let caller = MediaSession::new(CallKeys::derive(&o, &a, &id, Role::Caller), pad).unwrap();
        let callee = MediaSession::new(CallKeys::derive(&o, &a, &id, Role::Callee), pad).unwrap();
        (caller, callee)
    }

    #[test]
    fn ida_e_volta_com_tamanho_fixo() {
        let (mut a, mut b) = pair(128);
        assert_eq!(a.room_id(), b.room_id());
        assert_eq!(a.relay_key(), b.relay_key());
        let p1 = a.seal(b"ola").unwrap();
        let p2 = a.seal(&[7u8; 126]).unwrap();
        let p3 = a.seal(&[]).unwrap();
        assert_eq!((p1.len(), p2.len(), p3.len()), (169, 169, 169));
        assert_eq!(b.open(&p1).unwrap(), (0, b"ola".to_vec()));
        assert_eq!(b.open(&p2).unwrap(), (1, vec![7u8; 126]));
        assert_eq!(b.open(&p3).unwrap(), (2, vec![]));
        let r = b.seal(b"resposta").unwrap();
        assert_eq!(a.open(&r).unwrap(), (0, b"resposta".to_vec()));
    }

    #[test]
    fn frame_grande_demais_e_recusado() {
        let (mut a, _) = pair(64);
        assert_eq!(a.seal(&[0u8; 63]), Err(CallError::FrameTooLarge));
        assert!(MediaSession::new(CallKeys::derive(&[0; 32], &[0; 32], &[0; 16], Role::Caller), 8).is_err());
    }

    #[test]
    fn repetido_recusado_fora_de_ordem_aceite() {
        let (mut a, mut b) = pair(64);
        let ps: Vec<_> = (0..5).map(|_| a.seal(b"x").unwrap()).collect();
        assert_eq!(b.open(&ps[3]).unwrap().0, 3);
        assert_eq!(b.open(&ps[0]).unwrap().0, 0);
        assert_eq!(b.open(&ps[3]), Err(CallError::Replay));
        assert_eq!(b.open(&ps[0]), Err(CallError::Replay));
        assert_eq!(b.open(&ps[4]).unwrap().0, 4);
        assert_eq!(b.open(&ps[1]).unwrap().0, 1);
    }

    #[test]
    fn muito_antigo_recusado() {
        let (mut a, mut b) = pair(64);
        let old = a.seal(b"velho").unwrap();
        for _ in 0..REPLAY_WINDOW + 5 {
            let p = a.seal(b"x").unwrap();
            b.open(&p).unwrap();
        }
        assert_eq!(b.open(&old), Err(CallError::Replay));
    }

    #[test]
    fn adulterado_recusado_e_janela_nao_avanca() {
        let (mut a, mut b) = pair(64);
        let p = a.seal(b"ola").unwrap();
        for i in [0usize, 5, 12, HEADER_LEN + 3, p.len() - RELAY_TAG_LEN - 1] {
            let mut t = p.clone();
            t[i] ^= 1;
            assert!(b.open(&t).is_err(), "byte {i}");
        }
        // O original continua a ser aceite: os adulterados nao fizeram avancar a janela.
        assert_eq!(b.open(&p).unwrap().1, b"ola".to_vec());
    }

    #[test]
    fn pacote_refletido_ou_de_outra_chamada_recusado() {
        let (mut a, mut b) = pair(64);
        let p = a.seal(b"ola").unwrap();
        // Devolvido a quem o enviou: sentido errado.
        assert_eq!(a.open(&p), Err(CallError::WrongRoom));
        // Outra chamada (segredo diferente).
        let mut c = MediaSession::new(CallKeys::derive(&[9; 32], &[2; 32], &[3; 16], Role::Callee), 64).unwrap();
        assert_eq!(c.open(&p), Err(CallError::WrongRoom));
        assert!(b.open(&p).is_ok());
    }

    #[test]
    fn etiqueta_do_relay() {
        let (mut a, b) = pair(64);
        let p = a.seal(b"ola").unwrap();
        let h = parse_header(&p).unwrap();
        assert_eq!((h.role, h.room_id, h.counter), (Role::Caller, b.room_id(), 0));
        assert!(relay_tag_valid(&b.relay_key(), &p));
        let mut t = p.clone();
        t[20] ^= 1;
        assert!(!relay_tag_valid(&b.relay_key(), &t));
        assert!(!relay_tag_valid(&[0u8; 32], &p));
        assert!(parse_header(&p[..20]).is_none());
    }

    #[test]
    fn sala_so_depende_da_oferta() {
        let a = CallKeys::derive(&[1; 32], &[2; 32], &[3; 16], Role::Caller);
        let b = CallKeys::derive(&[1; 32], &[9; 32], &[3; 16], Role::Callee);
        let r = RoomKeys::derive(&[1; 32], &[3; 16]);
        assert_eq!((a.room_id(), a.relay_key()), (r.room_id, r.relay_key));
        assert_eq!(b.room_id(), r.room_id);
    }

    #[test]
    fn pedido_e_resposta_de_entrada() {
        let r = RoomKeys::derive(&[1; 32], &[3; 16]);
        let p = join_payload(Role::Callee, &r);
        let (role, back) = parse_join_payload(&p).unwrap();
        assert_eq!((role, back.room_id, back.relay_key), (Role::Callee, r.room_id, r.relay_key));
        assert!(parse_join_payload(&p[1..]).is_none());
        let ok = join_ok(&r, true);
        assert_eq!(parse_join_ok(&r, &ok), Some(true));
        assert_eq!(parse_join_ok(&r, &join_ok(&r, false)), Some(false));
        let mut bad = ok;
        bad[9] = 0;
        assert_eq!(parse_join_ok(&r, &bad), None);
        let other = RoomKeys::derive(&[2; 32], &[3; 16]);
        assert_eq!(parse_join_ok(&other, &ok), None);
        assert!(JOIN_OK_LEN < 1 + 32 + JOIN_PAYLOAD_LEN + 16, "resposta menor que o pedido");
    }

    #[test]
    fn segredos_diferentes_dao_chaves_diferentes() {
        let k1 = CallKeys::derive(&[1; 32], &[2; 32], &[3; 16], Role::Caller);
        let k2 = CallKeys::derive(&[4; 32], &[2; 32], &[3; 16], Role::Caller);
        let k3 = CallKeys::derive(&[1; 32], &[2; 32], &[5; 16], Role::Caller);
        assert_ne!(k1.room_id(), k2.room_id());
        assert_ne!(k1.room_id(), k3.room_id());
        assert_ne!(k1.relay_key(), k2.relay_key());
        // A resposta muda as chaves do audio (o outro lado tambem contribui).
        let k4 = CallKeys::derive(&[1; 32], &[7; 32], &[3; 16], Role::Caller);
        assert_ne!(k1.send_key, k4.send_key);
    }
}
