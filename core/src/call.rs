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
//!
//! LIGACAO DIRETA (P2P, fase 5, so entre contactos mutuos; a regra esta em
//! `qt_ffi::call`). Os pacotes de audio sao os mesmos; so muda o destino.
//!   * ENDERECO VISTO PELO RELAY: `[1 0x22][8 room_id][8 txid][23 zeros][8 etiqueta]`
//!     (48 bytes, etiqueta com a `relay_key`). O relay so responde se a sala existir:
//!     `[1 0x23][8 room_id][8 txid][19 endereco][8 etiqueta]` (44 bytes, menor que o
//!     pedido). Nao revela nada novo ao relay: ja ve o IP.
//!   * SONDAS do caminho direto: `[1 0x30|papel][8 room_id][8 txid][8 etiqueta]` e a
//!     resposta `[1 0x32|papel]...` com o mesmo txid. A etiqueta usa a `p2p_key`, que
//!     sai da oferta E da resposta (o relay nao a tem: nao consegue forjar sondas).
//!   * ENDERECO (19 bytes): `[1 familia 4|6][16 IP, o IPv4 nos 4 primeiros e zeros][2 porta BE]`.
//!
//! VIDEO (fase 5a). Um fluxo a mais dentro da chamada, pelo mesmo caminho do audio (direto
//! ou relay). Chaves proprias, da oferta E da resposta, com outra etiqueta
//! (`"QT-CALL-v1 video caller->callee"` / `"... callee->caller"`): o contador e a janela
//! anti-repeticao sao outros, e um pacote de audio nunca passa por video nem o contrario.
//!   * PACOTE: `[1 0x40|papel][8 room_id][8 contador BE][AEAD([2 tamanho][conteudo][zeros]) + 16][8 etiqueta]`.
//!     O texto cifrado tem um multiplo de 256 bytes, no maximo 1184 (pacote de 1225 bytes,
//!     cabe no MTU minimo do IPv6). O conteudo (pedacos de imagens, relatorios, pedidos de
//!     imagem completa) e do motor (`qt_call::video`); aqui so se cifra.
//!   * AVISO DO RELAY: `[1 0x24][8 room_id][1 modo][8 etiqueta relay_key]` (18 bytes): 0 video
//!     normal, 1 so ate 240p (travao do mes), 2 sem video pelo relay (teto da conta).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

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
/// Pedido "que endereco ves?" ao relay e a resposta (ligacao direta).
pub const ADDR_KIND: u8 = 0x22;
pub const ADDR_OK_KIND: u8 = 0x23;
/// Endereco IP + porta codificado (ver `encode_addr`).
pub const ADDR_BYTES: usize = 1 + 16 + 2;
/// O pedido e maior que a resposta: o relay nunca amplifica.
pub const ADDR_LEN: usize = 48;
pub const ADDR_OK_LEN: usize = 1 + ROOM_ID_LEN + 8 + ADDR_BYTES + RELAY_TAG_LEN;
/// Pacote de video; o bit 0 e o papel de quem envia.
pub const VIDEO_KIND: u8 = 0x40;
/// O texto cifrado de um pacote de video cresce de 256 em 256 bytes...
pub const VIDEO_PAD_STEP: usize = 256;
/// ... ate este maximo (pacote de 1225 bytes: cabe nos 1232 do MTU minimo do IPv6).
pub const VIDEO_MAX_PLAIN: usize = 1184;
/// Maior conteudo de um pacote de video (o texto cifrado leva 2 bytes de tamanho).
pub const VIDEO_MAX_BODY: usize = VIDEO_MAX_PLAIN - 2;
pub const VIDEO_MIN_PACKET: usize = HEADER_LEN + VIDEO_PAD_STEP + AEAD_TAG_LEN + RELAY_TAG_LEN;
pub const VIDEO_MAX_PACKET: usize = HEADER_LEN + VIDEO_MAX_PLAIN + AEAD_TAG_LEN + RELAY_TAG_LEN;
/// Aviso do relay sobre o video da sala (ver `RelayVideo`).
pub const RELAY_VIDEO_KIND: u8 = 0x24;
pub const RELAY_VIDEO_LEN: usize = 1 + ROOM_ID_LEN + 1 + RELAY_TAG_LEN;
/// Sonda do caminho direto e a resposta (o bit 0 e o papel de quem envia).
pub const PROBE_KIND: u8 = 0x30;
pub const PROBE_ACK_KIND: u8 = 0x32;
pub const PROBE_LEN: usize = 1 + ROOM_ID_LEN + 8 + RELAY_TAG_LEN;
/// Janela anti-repeticao (pacotes).
pub const REPLAY_WINDOW: u64 = 1024;
// A copia que vem pelo relay chega 50-150 ms (3-8 pacotes) depois da direta: se o direto
// perdeu o pacote, essa copia tem de caber na janela (nunca "demasiado antiga").
const _: () = assert!(REPLAY_WINDOW >= 64);
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
    /// Etiquetas das sondas do caminho direto. Sai da oferta E da resposta: o relay
    /// (que so conhece a oferta atraves da `relay_key`) nao a consegue calcular.
    p2p_key: [u8; 32],
    /// Video: uma chave por sentido, separadas das do audio.
    video_send_key: [u8; 32],
    video_recv_key: [u8; 32],
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
        let mut p2p_key = [0u8; 32];
        hk.expand(b"QT-CALL-v1 p2p", &mut p2p_key).expect("hkdf p2p");
        let mut v_to_callee = [0u8; 32];
        let mut v_to_caller = [0u8; 32];
        hk.expand(b"QT-CALL-v1 video caller->callee", &mut v_to_callee).expect("hkdf video");
        hk.expand(b"QT-CALL-v1 video callee->caller", &mut v_to_caller).expect("hkdf video");
        let (video_send_key, video_recv_key) = match role {
            Role::Caller => (v_to_callee, v_to_caller),
            Role::Callee => (v_to_caller, v_to_callee),
        };
        v_to_callee.zeroize();
        v_to_caller.zeroize();
        let (send_key, recv_key) = match role {
            Role::Caller => (to_callee, to_caller),
            Role::Callee => (to_caller, to_callee),
        };
        to_callee.zeroize();
        to_caller.zeroize();
        CallKeys { room_id, relay_key, send_key, recv_key, p2p_key, video_send_key, video_recv_key, role }
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

/// Le o cabecalho de um pacote de VIDEO (para o relay encaminhar). Nao verifica nada criptografico.
pub fn parse_video_header(packet: &[u8]) -> Option<MediaHeader> {
    if !(VIDEO_MIN_PACKET..=VIDEO_MAX_PACKET).contains(&packet.len()) || !(packet.len() - VIDEO_MIN_PACKET).is_multiple_of(VIDEO_PAD_STEP) && packet.len() != VIDEO_MAX_PACKET {
        return None;
    }
    let role = match packet[0] {
        k if k == VIDEO_KIND => Role::Caller,
        k if k == VIDEO_KIND | 1 => Role::Callee,
        _ => return None,
    };
    let mut room_id = [0u8; ROOM_ID_LEN];
    room_id.copy_from_slice(&packet[1..1 + ROOM_ID_LEN]);
    let counter = u64::from_be_bytes(packet[1 + ROOM_ID_LEN..HEADER_LEN].try_into().ok()?);
    Some(MediaHeader { role, room_id, counter })
}

/// E um pacote de video (de qualquer lado)? Barato, sem chaves.
pub fn is_video(p: &[u8]) -> bool {
    p.first().is_some_and(|k| k & !1 == VIDEO_KIND)
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

/// O que o relay deixa passar de video nesta sala.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayVideo {
    /// Normal (ate 360p).
    Full = 0,
    /// Travao do mes (o relay ja enviou demasiado): so ate 240p, para todos.
    Cap240 = 1,
    /// Sem video pelo relay (teto da conta): so voz. Em ligacao direta continua.
    Denied = 2,
}

/// Aviso do relay sobre o video desta sala (mais pequeno do que qualquer pacote de video).
pub fn relay_video_notice(room_id: &[u8; ROOM_ID_LEN], relay_key: &[u8; 32], mode: RelayVideo) -> [u8; RELAY_VIDEO_LEN] {
    let mut p = [0u8; RELAY_VIDEO_LEN];
    p[0] = RELAY_VIDEO_KIND;
    p[1..1 + ROOM_ID_LEN].copy_from_slice(room_id);
    p[1 + ROOM_ID_LEN] = mode as u8;
    let tag = relay_tag(relay_key, &p[..RELAY_VIDEO_LEN - RELAY_TAG_LEN]);
    p[RELAY_VIDEO_LEN - RELAY_TAG_LEN..].copy_from_slice(&tag);
    p
}

/// O telemovel le o aviso do relay.
pub fn parse_relay_video_notice(room: &RoomKeys, p: &[u8]) -> Option<RelayVideo> {
    if p.len() != RELAY_VIDEO_LEN || p[0] != RELAY_VIDEO_KIND || p[1..1 + ROOM_ID_LEN] != room.room_id {
        return None;
    }
    if !relay_tag_valid(&room.relay_key, p) {
        return None;
    }
    match p[1 + ROOM_ID_LEN] {
        0 => Some(RelayVideo::Full),
        1 => Some(RelayVideo::Cap240),
        2 => Some(RelayVideo::Denied),
        _ => None,
    }
}

/// Codifica um endereco em 19 bytes (um IPv4 dentro de IPv6, `::ffff:a.b.c.d`, vai como IPv4).
pub fn encode_addr(a: &SocketAddr) -> [u8; ADDR_BYTES] {
    let mut out = [0u8; ADDR_BYTES];
    match a.ip().to_canonical() {
        IpAddr::V4(v4) => {
            out[0] = 4;
            out[1..5].copy_from_slice(&v4.octets());
        }
        IpAddr::V6(v6) => {
            out[0] = 6;
            out[1..17].copy_from_slice(&v6.octets());
        }
    }
    out[17..].copy_from_slice(&a.port().to_be_bytes());
    out
}

/// Le um endereco de 19 bytes. Recusa porta 0, familia desconhecida e IPv4 com lixo.
pub fn decode_addr(b: &[u8]) -> Option<SocketAddr> {
    if b.len() != ADDR_BYTES {
        return None;
    }
    let port = u16::from_be_bytes([b[17], b[18]]);
    if port == 0 {
        return None;
    }
    let ip = match b[0] {
        4 if b[5..17].iter().all(|x| *x == 0) => IpAddr::V4(Ipv4Addr::new(b[1], b[2], b[3], b[4])),
        6 => {
            let o: [u8; 16] = b[1..17].try_into().ok()?;
            IpAddr::V6(Ipv6Addr::from(o)).to_canonical()
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

/// Pedido ao relay: "que endereco ves?" (para o telemovel saber o seu IP publico e porta).
pub fn addr_request(room: &RoomKeys, txid: u64) -> [u8; ADDR_LEN] {
    let mut p = [0u8; ADDR_LEN];
    p[0] = ADDR_KIND;
    p[1..1 + ROOM_ID_LEN].copy_from_slice(&room.room_id);
    p[1 + ROOM_ID_LEN..1 + ROOM_ID_LEN + 8].copy_from_slice(&txid.to_be_bytes());
    let tag = relay_tag(&room.relay_key, &p[..ADDR_LEN - RELAY_TAG_LEN]);
    p[ADDR_LEN - RELAY_TAG_LEN..].copy_from_slice(&tag);
    p
}

/// O relay le a sala de um pedido de endereco (a etiqueta verifica-se depois, com a chave da sala).
pub fn addr_request_room(p: &[u8]) -> Option<[u8; ROOM_ID_LEN]> {
    if p.len() != ADDR_LEN || p[0] != ADDR_KIND || p[1 + ROOM_ID_LEN + 8..ADDR_LEN - RELAY_TAG_LEN].iter().any(|x| *x != 0) {
        return None;
    }
    p[1..1 + ROOM_ID_LEN].try_into().ok()
}

/// O relay responde com o endereco de onde veio o pedido. `None`: pedido invalido.
pub fn addr_ok(relay_key: &[u8; 32], request: &[u8], seen: &SocketAddr) -> Option<[u8; ADDR_OK_LEN]> {
    addr_request_room(request)?;
    if !relay_tag_valid(relay_key, request) {
        return None;
    }
    let mut p = [0u8; ADDR_OK_LEN];
    p[0] = ADDR_OK_KIND;
    p[1..1 + ROOM_ID_LEN + 8].copy_from_slice(&request[1..1 + ROOM_ID_LEN + 8]);
    p[1 + ROOM_ID_LEN + 8..1 + ROOM_ID_LEN + 8 + ADDR_BYTES].copy_from_slice(&encode_addr(seen));
    let tag = relay_tag(relay_key, &p[..ADDR_OK_LEN - RELAY_TAG_LEN]);
    p[ADDR_OK_LEN - RELAY_TAG_LEN..].copy_from_slice(&tag);
    Some(p)
}

/// O telemovel le a resposta: `(txid, o meu endereco visto pelo relay)`.
pub fn parse_addr_ok(room: &RoomKeys, p: &[u8]) -> Option<(u64, SocketAddr)> {
    if p.len() != ADDR_OK_LEN || p[0] != ADDR_OK_KIND || p[1..1 + ROOM_ID_LEN] != room.room_id {
        return None;
    }
    if !relay_tag_valid(&room.relay_key, p) {
        return None;
    }
    let txid = u64::from_be_bytes(p[1 + ROOM_ID_LEN..1 + ROOM_ID_LEN + 8].try_into().ok()?);
    let addr = decode_addr(&p[1 + ROOM_ID_LEN + 8..1 + ROOM_ID_LEN + 8 + ADDR_BYTES])?;
    Some((txid, addr))
}

/// Sonda ou resposta do caminho direto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// "Chegas-me por aqui?" (responder com `Ack` e o mesmo txid, para quem a enviou).
    Ping(u64),
    /// Resposta a uma sonda minha: o caminho funciona nos dois sentidos.
    Ack(u64),
}

/// Chaves das sondas do caminho direto de UMA chamada (so existem depois de atender).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct PathKeys {
    room_id: [u8; ROOM_ID_LEN],
    p2p_key: [u8; 32],
    #[zeroize(skip)]
    role: Role,
}

impl PathKeys {
    pub fn probe(&self, p: Probe) -> [u8; PROBE_LEN] {
        let (kind, txid) = match p {
            Probe::Ping(t) => (PROBE_KIND, t),
            Probe::Ack(t) => (PROBE_ACK_KIND, t),
        };
        let mut out = [0u8; PROBE_LEN];
        out[0] = kind | self.role.bit();
        out[1..1 + ROOM_ID_LEN].copy_from_slice(&self.room_id);
        out[1 + ROOM_ID_LEN..1 + ROOM_ID_LEN + 8].copy_from_slice(&txid.to_be_bytes());
        let tag = relay_tag(&self.p2p_key, &out[..PROBE_LEN - RELAY_TAG_LEN]);
        out[PROBE_LEN - RELAY_TAG_LEN..].copy_from_slice(&tag);
        out
    }

    /// Le uma sonda do OUTRO lado (uma minha devolvida e recusada). Comparacao em tempo constante.
    pub fn parse(&self, p: &[u8]) -> Option<Probe> {
        if p.len() != PROBE_LEN || p[1..1 + ROOM_ID_LEN] != self.room_id {
            return None;
        }
        let other = self.role.other().bit();
        let is_ping = p[0] == PROBE_KIND | other;
        if !is_ping && p[0] != PROBE_ACK_KIND | other {
            return None;
        }
        if !relay_tag_valid(&self.p2p_key, p) {
            return None;
        }
        let txid = u64::from_be_bytes(p[1 + ROOM_ID_LEN..1 + ROOM_ID_LEN + 8].try_into().ok()?);
        Some(if is_ping { Probe::Ping(txid) } else { Probe::Ack(txid) })
    }
}

/// E uma sonda (de qualquer lado)? Barato, sem chaves: so para separar do audio.
pub fn is_probe(p: &[u8]) -> bool {
    p.len() == PROBE_LEN && matches!(p[0] & !1, PROBE_KIND | PROBE_ACK_KIND)
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

    /// Cifra e decifra o VIDEO desta chamada (chaves e contadores proprios).
    pub fn video(&self) -> VideoSession {
        VideoSession {
            room_id: self.keys.room_id,
            relay_key: self.keys.relay_key,
            send_key: self.keys.video_send_key,
            recv_key: self.keys.video_recv_key,
            role: self.keys.role,
            send_counter: 0,
            replay: ReplayWindow::new(),
        }
    }

    /// Chaves das sondas do caminho direto (para o motor; nunca saem do Rust).
    pub fn path_keys(&self) -> PathKeys {
        PathKeys { room_id: self.keys.room_id, p2p_key: self.keys.p2p_key, role: self.keys.role }
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

/// Tamanho do texto cifrado de um pacote de video com `body` bytes de conteudo.
pub fn video_plain_len(body: usize) -> usize {
    (2 + body).div_ceil(VIDEO_PAD_STEP).saturating_mul(VIDEO_PAD_STEP).min(VIDEO_MAX_PLAIN)
}

/// Tamanho do pacote de video com `body` bytes de conteudo.
pub fn video_packet_len(body: usize) -> usize {
    HEADER_LEN + video_plain_len(body) + AEAD_TAG_LEN + RELAY_TAG_LEN
}

/// Cifra e decifra os pacotes de VIDEO de uma chamada.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct VideoSession {
    room_id: [u8; ROOM_ID_LEN],
    relay_key: [u8; 32],
    send_key: [u8; 32],
    recv_key: [u8; 32],
    #[zeroize(skip)]
    role: Role,
    send_counter: u64,
    #[zeroize(skip)]
    replay: ReplayWindow,
}

impl VideoSession {
    /// Cifra um conteudo (ate `VIDEO_MAX_BODY` bytes). O tamanho do pacote so revela o
    /// conteudo arredondado a 256 bytes.
    pub fn seal(&mut self, body: &[u8]) -> Result<Vec<u8>, CallError> {
        if body.len() > VIDEO_MAX_BODY {
            return Err(CallError::FrameTooLarge);
        }
        if self.send_counter >= MAX_COUNTER {
            return Err(CallError::Exhausted);
        }
        let counter = self.send_counter;
        self.send_counter += 1;
        let plain_len = video_plain_len(body.len());
        let mut out = Vec::with_capacity(HEADER_LEN + plain_len + AEAD_TAG_LEN + RELAY_TAG_LEN);
        out.push(VIDEO_KIND | self.role.bit());
        out.extend_from_slice(&self.room_id);
        out.extend_from_slice(&counter.to_be_bytes());
        let mut plain = vec![0u8; plain_len];
        plain[..2].copy_from_slice(&(body.len() as u16).to_be_bytes());
        plain[2..2 + body.len()].copy_from_slice(body);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.send_key));
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce(counter)), Payload { msg: &plain, aad: &out })
            .map_err(|_| CallError::Decrypt)?;
        plain.zeroize();
        out.extend_from_slice(&ct);
        let tag = relay_tag(&self.relay_key, &out);
        out.extend_from_slice(&tag);
        Ok(out)
    }

    /// Decifra um pacote de video do outro lado: (contador, conteudo). `Replay` = copia
    /// repetida (ex.: o mesmo pacote pelos dois caminhos).
    pub fn open(&mut self, packet: &[u8]) -> Result<(u64, Vec<u8>), CallError> {
        let h = parse_video_header(packet).ok_or(CallError::Malformed)?;
        if h.role != self.role.other() || h.room_id != self.room_id {
            return Err(CallError::WrongRoom);
        }
        if h.counter >= MAX_COUNTER {
            return Err(CallError::Malformed);
        }
        if !self.replay.check(h.counter) {
            return Err(CallError::Replay);
        }
        let aad = &packet[..HEADER_LEN];
        let ct = &packet[HEADER_LEN..packet.len() - RELAY_TAG_LEN];
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.recv_key));
        let mut plain = cipher
            .decrypt(Nonce::from_slice(&nonce(h.counter)), Payload { msg: ct, aad })
            .map_err(|_| CallError::Decrypt)?;
        self.replay.update(h.counter);
        let len = u16::from_be_bytes([plain[0], plain[1]]) as usize;
        if len > plain.len() - 2 {
            plain.zeroize();
            return Err(CallError::Malformed);
        }
        let body = plain[2..2 + len].to_vec();
        plain.zeroize();
        Ok((h.counter, body))
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
    fn copia_atrasada_pelo_relay_cabe_na_janela() {
        // Direto: chegam 0..=70 menos o 3 e o 7 (perdidos). Pelo relay chegam depois as
        // copias de todos: as do 3 e do 7 (8 e 64 pacotes atras) sao aceites; o resto e
        // repetido.
        let (mut a, mut b) = pair(64);
        let ps: Vec<_> = (0..71).map(|_| a.seal(b"x").unwrap()).collect();
        for (i, p) in ps.iter().enumerate().take(11) {
            if i != 3 && i != 7 {
                b.open(p).unwrap();
            }
        }
        assert_eq!(b.open(&ps[3]).unwrap().0, 3, "8 pacotes atras");
        for p in &ps[11..71] {
            b.open(p).unwrap();
        }
        assert_eq!(b.open(&ps[7]).unwrap().0, 7, "64 pacotes atras");
        for p in &ps[..71] {
            assert_eq!(b.open(p), Err(CallError::Replay), "a segunda copia e sempre recusada");
        }
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
    fn enderecos_ida_e_volta() {
        for a in ["1.2.3.4:443", "[2001:db8::7]:50000", "192.168.1.10:9"] {
            let a: SocketAddr = a.parse().unwrap();
            assert_eq!(decode_addr(&encode_addr(&a)), Some(a));
        }
        // IPv4 dentro de IPv6 (socket dual) sai como IPv4.
        let m: SocketAddr = "[::ffff:1.2.3.4]:5".parse().unwrap();
        assert_eq!(decode_addr(&encode_addr(&m)), Some("1.2.3.4:5".parse().unwrap()));
        let mut b = encode_addr(&"1.2.3.4:5".parse().unwrap());
        b[9] = 1;
        assert_eq!(decode_addr(&b), None, "IPv4 com lixo");
        let mut b = encode_addr(&"1.2.3.4:5".parse().unwrap());
        b[17] = 0;
        b[18] = 0;
        assert_eq!(decode_addr(&b), None, "porta 0");
        b[0] = 5;
        assert_eq!(decode_addr(&b), None);
    }

    #[test]
    fn endereco_visto_pelo_relay() {
        let r = RoomKeys::derive(&[1; 32], &[3; 16]);
        let req = addr_request(&r, 77);
        assert_eq!(addr_request_room(&req), Some(r.room_id));
        let seen: SocketAddr = "[2001:db8::1]:40000".parse().unwrap();
        let ok = addr_ok(&r.relay_key, &req, &seen).unwrap();
        assert!(ok.len() < req.len(), "resposta menor que o pedido");
        assert_eq!(parse_addr_ok(&r, &ok), Some((77, seen)));
        // Etiqueta errada (quem nao conhece a sala): o relay nao responde.
        assert!(addr_ok(&[9u8; 32], &req, &seen).is_none());
        let mut bad = req;
        bad[20] = 1;
        assert!(addr_request_room(&bad).is_none());
        // Resposta adulterada ou de outra sala: recusada.
        let mut t = ok;
        t[20] ^= 1;
        assert!(parse_addr_ok(&r, &t).is_none());
        assert!(parse_addr_ok(&RoomKeys::derive(&[2; 32], &[3; 16]), &ok).is_none());
    }

    #[test]
    fn sondas_do_caminho_direto() {
        let (a, b) = pair(64);
        let (pa, pb) = (a.path_keys(), b.path_keys());
        let ping = pa.probe(Probe::Ping(5));
        assert!(is_probe(&ping) && !is_probe(&ping[1..]));
        assert_eq!(pb.parse(&ping), Some(Probe::Ping(5)));
        let ack = pb.probe(Probe::Ack(5));
        assert_eq!(pa.parse(&ack), Some(Probe::Ack(5)));
        // Uma sonda minha devolvida (refletida) nao conta.
        assert_eq!(pa.parse(&ping), None);
        // Adulterada ou de outra chamada: recusada.
        let mut t = ping;
        t[12] ^= 1;
        assert_eq!(pb.parse(&t), None);
        let mut c = MediaSession::new(CallKeys::derive(&[1; 32], &[8; 32], &[3; 16], Role::Callee), 64).unwrap();
        assert_eq!(c.path_keys().parse(&ping), None, "mesma sala, outra resposta: outra p2p_key");
        // O relay conhece a relay_key, nao a p2p_key: nao consegue forjar sondas.
        let r = RoomKeys::derive(&[1; 32], &[3; 16]);
        let mut forged = ping;
        let tag = relay_tag(&r.relay_key, &forged[..PROBE_LEN - RELAY_TAG_LEN]);
        forged[PROBE_LEN - RELAY_TAG_LEN..].copy_from_slice(&tag);
        assert_eq!(pb.parse(&forged), None);
        // Nao se confundem com audio.
        assert!(parse_header(&ping).is_none());
        let _ = c.seal(b"x");
    }

    #[test]
    fn video_ida_e_volta_com_enchimento_a_256() {
        let (a, b) = pair(64);
        let (mut va, mut vb) = (a.video(), b.video());
        for (n, plain) in [(0usize, 256usize), (10, 256), (254, 256), (255, 512), (700, 768), (1000, 1024), (VIDEO_MAX_BODY, VIDEO_MAX_PLAIN)] {
            let body = vec![n as u8; n];
            let p = va.seal(&body).unwrap();
            assert_eq!(p.len(), HEADER_LEN + plain + 16 + RELAY_TAG_LEN, "conteudo de {n}");
            assert!(p.len() <= 1232, "cabe no MTU minimo do IPv6");
            assert_eq!(parse_video_header(&p).map(|h| h.role), Some(Role::Caller));
            assert!(is_video(&p) && parse_header(&p).is_none(), "nao e audio");
            assert_eq!(vb.open(&p).unwrap().1, body);
            assert!(relay_tag_valid(&b.relay_key(), &p));
        }
        assert_eq!(va.seal(&vec![0; VIDEO_MAX_BODY + 1]), Err(CallError::FrameTooLarge));
        let r = vb.seal(b"resposta").unwrap();
        assert_eq!(va.open(&r).unwrap(), (0, b"resposta".to_vec()));
    }

    #[test]
    fn video_e_audio_nao_se_misturam() {
        let (mut a, mut b) = pair(128);
        let (mut va, mut vb) = (a.video(), b.video());
        let v = va.seal(b"imagem").unwrap();
        let au = a.seal(b"voz").unwrap();
        // Chaves e formatos diferentes: um nunca abre como o outro.
        assert!(b.open(&v).is_err());
        assert!(vb.open(&au).is_err());
        // Contadores separados: o video comeca no 0 mesmo depois de audio.
        assert_eq!(vb.open(&v).unwrap().0, 0);
        assert_eq!(b.open(&au).unwrap().0, 0);
        // Repetido, refletido e adulterado: recusados.
        assert_eq!(vb.open(&v), Err(CallError::Replay));
        assert_eq!(va.open(&v), Err(CallError::WrongRoom));
        let mut t = va.seal(b"x").unwrap();
        t[30] ^= 1;
        assert!(vb.open(&t).is_err());
        // Tamanhos que nao sao degraus de 256 nao passam no relay.
        let ok = va.seal(b"y").unwrap();
        assert!(parse_video_header(&ok[..ok.len() - 1]).is_none());
        // Outra resposta (outra chamada na mesma sala): outra chave de video.
        let c = MediaSession::new(CallKeys::derive(&[1; 32], &[9; 32], &[3; 16], Role::Callee), 64).unwrap();
        assert_eq!(c.video().open(&ok), Err(CallError::Decrypt));
    }

    #[test]
    fn aviso_do_relay_sobre_o_video() {
        let r = RoomKeys::derive(&[1; 32], &[3; 16]);
        for m in [RelayVideo::Full, RelayVideo::Cap240, RelayVideo::Denied] {
            let p = relay_video_notice(&r.room_id, &r.relay_key, m);
            assert_eq!(parse_relay_video_notice(&r, &p), Some(m));
            assert!(p.len() < VIDEO_MIN_PACKET, "menor do que qualquer pacote de video");
        }
        let mut p = relay_video_notice(&r.room_id, &r.relay_key, RelayVideo::Denied);
        p[9] = 1;
        assert_eq!(parse_relay_video_notice(&r, &p), None, "adulterado");
        let other = RoomKeys::derive(&[2; 32], &[3; 16]);
        assert_eq!(parse_relay_video_notice(&other, &relay_video_notice(&r.room_id, &r.relay_key, RelayVideo::Full)), None);
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
