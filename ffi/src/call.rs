//! Chamadas de voz 1 a 1: sinalizacao dentro do Double Ratchet (tipo 11).
//!
//! DESENHO (aprovado pelo Nuno a 19/10/2026, ver docs/protocol-spec.md, seccao 18):
//!   * A oferta, o "a tocar", a resposta e o desligar vao como mensagens cifradas
//!     iguais as outras (envelope v3 com enchimento): o relay nao sabe que e uma
//!     chamada pelo conteudo. A oferta e o cancelamento antes de atender tocam a
//!     campainha marcada como chamada (`CallWake`): assim uma app fechada com o
//!     bloqueio ativo toca ou para sem decifrar nada (aprovado a 19/10/2026; o relay e a
//!     Google ficam a saber o momento de uma chamada). O resto vai `silent`.
//!   * Cada lado contribui 32 bytes aleatorios (oferta e resposta). As chaves do
//!     audio, o codigo da sala e a chave do relay saem dai (`qt_core::call`).
//!   * Tudo fica SO em memoria (nada em disco). Uma chamada de cada vez.
//!   * O audio comeca SEMPRE pelo relay. LIGACAO DIRETA (P2P, fase 5) so depois de
//!     atender e so se, DESTE lado, o contacto estiver verificado (`is_verified`) e a
//!     opcao "chamadas sempre pelo relay" estiver desligada (`p2p_allowed`):
//!       1. cada lado que cumpre a regra envia "posso" (op 5, sem enderecos);
//!       2. so quem enviou "posso" E recebeu o "posso" do outro envia os seus enderecos
//!          (op 6). Assim nenhum endereco sai para quem nao verificou quem liga, nem para
//!          quem tem "sempre pelo relay": esse lado nunca envia "posso";
//!       3. os enderecos recebidos so vao para o motor de quem enviou "posso" (a regra
//!          volta a ser verificada nesse momento).
//!   * O motor (`qt_call::engine`) tenta o caminho direto e, se falhar, fica no relay sem
//!     cortar a chamada. Uma versao antiga da app ignora as ops 5 e 6: fica no relay.
//!   * Ligamos os dois ao mesmo tempo um ao outro: fica a chamada com o call_id menor;
//!     quem tinha a outra recebe `Ended(Cancelled)` dela e `Incoming` da que fica.
//!   * Qualquer pessoa com sessao pode ligar (como pode escrever): e a app que decide
//!     se toca para quem nao esta nos contactos (senao, `call_hangup` = recusar).
//!
//! FORMATO: payload v1, tipo 11; `msg_id` = `call_id` (uma versao antiga da app mostra
//! "atualiza a app" uma vez por chamada). Corpo: [1 op][16 call_id] e:
//!   op 1 oferta:   [32 offer_secret][2 pad BE]
//!   op 2 resposta: [32 answer_secret]
//!   op 3 desligar: [1 motivo] (1 desligou, 2 recusou, 3 ocupado, 4 sem resposta, 5 cancelou)
//!                  (o 6, "chegou tarde", so existe localmente e nunca e enviado)
//!   op 4 a tocar:  (vazio)
//!   op 5 posso ligar direto: (vazio)               (so depois de atender)
//!   op 6 enderecos: [1 n, 1..=5][n x 19 endereco]  (`qt_core::call::encode_addr`)

use std::collections::VecDeque;
use std::net::SocketAddr;

use qt_core::call::{
    decode_addr, encode_addr, join_payload, parse_join_ok, CallKeys, MediaSession, Role, RoomKeys, ADDR_BYTES, JOIN_KIND,
    JOIN_PROLOGUE, MAX_PAD, MIN_PAD,
};
use qt_storage as st;
use qt_storage::messages as hm;
use zeroize::Zeroizing;

use crate::client::{bad, hex, unhex, QtClient, QtError};
use crate::history::{is_blocked_in, now_ms, CallWake, OutboxItem, ReceiveResult, HDR, PAYLOAD_V1};

pub(crate) const T_CALL: u8 = 11;
const OP_OFFER: u8 = 1;
const OP_ANSWER: u8 = 2;
const OP_HANGUP: u8 = 3;
const OP_RINGING: u8 = 4;
const OP_P2P_READY: u8 = 5;
const OP_P2P_ADDRS: u8 = 6;

/// Maximo de enderecos numa mensagem op 6: o corpo do tipo 11 tem no maximo 128 bytes
/// (`history.rs`), e 17 + 1 + 5 x 19 = 113. O motor da primeiro o IP publico.
pub const MAX_P2P_ADDRS: usize = 5;
const _: () = assert!(17 + 1 + MAX_P2P_ADDRS * ADDR_BYTES <= 128);
/// "Chamadas sempre pelo relay" (definicao deste telemovel).
const KV_RELAY_ONLY: &str = "calls_relay_only";
/// Contacto verificado: guarda a chave de identidade que foi verificada. Se a chave da
/// sessao deixar de ser esta, a verificacao deixa de valer (e a chamada fica no relay).
pub(crate) const KV_VERIFIED: &str = "verified:";

/// Uma oferta que chega mais tarde do que isto (ficou na fila do relay) ja nao toca:
/// fica como chamada perdida.
pub const OFFER_TTL_MS: i64 = 45_000;
/// Sem resposta ao fim disto, quem liga desiste (e quem recebe deixa de tocar).
pub const RING_TIMEOUT_MS: i64 = 45_000;
/// Tamanho fixo do frame cifrado (fase 1). A fase do audio afina este valor.
pub const DEFAULT_PAD: u16 = 128;
const MAX_EVENTS: usize = 64;
/// Chamadas terminadas de que ainda nos lembramos (para ignorar respostas atrasadas).
const MAX_ENDED: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CallEndReason {
    /// Alguem desligou uma chamada atendida.
    Hangup,
    /// Quem recebeu recusou.
    Rejected,
    /// Quem recebeu ja estava noutra chamada.
    Busy,
    /// Ninguem atendeu a tempo.
    NoAnswer,
    /// Quem ligou desistiu antes de atenderem (do lado de quem recebe: chamada perdida).
    Cancelled,
    /// A oferta chegou tarde demais (telemovel sem rede): chamada perdida.
    Expired,
}

impl CallEndReason {
    fn to_byte(self) -> u8 {
        match self {
            CallEndReason::Hangup => 1,
            CallEndReason::Rejected => 2,
            CallEndReason::Busy => 3,
            CallEndReason::NoAnswer => 4,
            CallEndReason::Cancelled => 5,
            CallEndReason::Expired => 6,
        }
    }
    fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            1 => CallEndReason::Hangup,
            2 => CallEndReason::Rejected,
            3 => CallEndReason::Busy,
            4 => CallEndReason::NoAnswer,
            5 => CallEndReason::Cancelled,
            6 => CallEndReason::Expired,
            _ => return None,
        })
    }
}

/// O que aconteceu as chamadas desde a ultima vez (`call_events`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CallEvent {
    /// Alguem esta a ligar-me: mostrar o ecra de chamada (atender / recusar).
    Incoming { call_id: String, contact_id: String },
    /// O telemovel de quem chamei esta a tocar.
    Ringing { call_id: String },
    /// Atenderam (do meu lado ou do outro): o audio pode comecar.
    Answered { call_id: String },
    /// A chamada acabou (ou nunca chegou a tocar).
    Ended { call_id: String, contact_id: String, reason: CallEndReason },
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CallStart {
    pub call_id: String,
    /// A oferta, ja na outbox: enviar ja ao relay (toca a campainha).
    pub item: OutboxItem,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct MediaFrame {
    /// Numero do pacote (para ordenar no jitter buffer).
    pub counter: u64,
    pub frame: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Liguei; espero resposta.
    Outgoing,
    /// Estao a ligar-me; ainda nao atendi.
    Incoming,
    /// Atendida: ha chaves de audio.
    Active,
}

struct Call {
    id: [u8; 16],
    contact: String,
    state: State,
    role: Role,
    offer_secret: Zeroizing<[u8; 32]>,
    /// Sala no relay (so depende da oferta): da para entrar enquanto toca.
    room: RoomKeys,
    pad: u16,
    since: i64,
    /// So para quem liga: o id da oferta na outbox (para a tirar se cancelar antes de sair).
    offer_outbox: Option<[u8; 16]>,
    media: Option<MediaSession>,
    /// O motor de audio (feature `calls`), se a app ja o arrancou: recebe as chaves do
    /// audio quando a chamada e atendida e para quando acaba.
    #[cfg(feature = "calls")]
    audio: Option<std::sync::Arc<crate::call_audio::CallAudio>>,
    p2p: P2p,
}

/// Ligacao direta desta chamada (ver o inicio do ficheiro).
#[derive(Default)]
struct P2p {
    /// Enviei "posso" (op 5): atendida e a regra cumprida deste lado.
    ready_sent: bool,
    /// O outro lado enviou "posso".
    peer_ready: bool,
    /// Ja enviei os meus enderecos (ou desisti: sem enderecos).
    addrs_sent: bool,
    /// Enderecos do outro lado a espera do motor de audio.
    peer_addrs: Option<Vec<SocketAddr>>,
}

impl Call {
    /// Chamada atendida: as chaves vao para o motor de audio (se ja existir) ou ficam aqui.
    fn set_media(&mut self, media: MediaSession) {
        #[cfg(feature = "calls")]
        if let Some(a) = &self.audio {
            a.attach(media);
            return;
        }
        self.media = Some(media);
    }
}

#[derive(Default)]
pub(crate) struct Calls {
    cur: Option<Call>,
    events: VecDeque<CallEvent>,
    ended: VecDeque<[u8; 16]>,
}

impl Calls {
    fn push(&mut self, e: CallEvent) {
        if self.events.len() >= MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(e);
    }
    fn remember_ended(&mut self, id: [u8; 16]) {
        if self.ended.len() >= MAX_ENDED {
            self.ended.pop_front();
        }
        self.ended.push_back(id);
    }
    fn end_current(&mut self, reason: CallEndReason) {
        if let Some(call) = self.cur.take() {
            #[cfg(feature = "calls")]
            if let Some(a) = &call.audio {
                a.stop();
            }
            self.remember_ended(call.id);
            self.push(CallEvent::Ended { call_id: hex(&call.id), contact_id: call.contact, reason });
        }
    }
}

fn random<const N: usize>() -> Result<[u8; N], QtError> {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).map_err(|_| QtError::Crypto)?;
    Ok(b)
}

fn parse_call_id(s: &str) -> Result<[u8; 16], QtError> {
    unhex(s).and_then(|b| b.try_into().ok()).ok_or_else(|| bad("call_id"))
}

fn encode(op: u8, call_id: &[u8; 16], extra: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HDR + 17 + extra.len());
    out.push(PAYLOAD_V1);
    out.push(T_CALL);
    out.extend_from_slice(call_id);
    out.extend_from_slice(&now_ms().to_be_bytes());
    out.push(op);
    out.extend_from_slice(call_id);
    out.extend_from_slice(extra);
    out
}

/// Uma oferta enviada a `sent_at` ainda pode tocar agora? Um relogio adiantado do
/// outro lado nao estica o prazo.
pub(crate) fn offer_fresh(sent_at: i64, now: i64) -> bool {
    sent_at.min(now) + OFFER_TTL_MS > now
}

/// Cifra uma mensagem de chamada para `cid` e poe-na na outbox. `wake`: a oferta e o
/// cancelamento antes de atender tocam a campainha marcada; o resto vai `silent`.
fn queue(client: &QtClient, c: &st::rusqlite::Connection, cid: &str, plain: &[u8], wake: Option<CallWake>) -> Result<OutboxItem, QtError> {
    let id = random::<16>()?;
    let blob = client.seal_with(c, cid, plain)?;
    let kind = match wake {
        Some(CallWake::Call) => hm::OUTBOX_CALL,
        Some(CallWake::CallEnd) => hm::OUTBOX_CALL_END,
        None => hm::OUTBOX_RECEIPT,
    };
    hm::outbox_put(c, &id, cid, kind, &blob)?;
    Ok(OutboxItem { outbox_id: hex(&id), contact_id: cid.to_string(), blob, silent: wake.is_none(), relay_id: None, wake })
}

/// "Chamadas sempre pelo relay" esta ligada neste telemovel?
pub(crate) fn relay_only(c: &st::rusqlite::Connection) -> Result<bool, QtError> {
    Ok(st::kv_get(c, KV_RELAY_ONLY)?.is_some_and(|v| v.first() == Some(&1)))
}

/// O contacto esta verificado, com a chave de identidade que tem AGORA na sessao?
pub(crate) fn is_verified(c: &st::rusqlite::Connection, cid: &str) -> Result<bool, QtError> {
    let Some(saved) = st::kv_get(c, &format!("{KV_VERIFIED}{cid}"))? else { return Ok(false) };
    let Some(meta) = st::load_session_meta(c, cid)? else { return Ok(false) };
    Ok(saved.len() == 32 && saved == meta.peer_identity_public)
}

/// Marca o contacto como verificado com a chave de identidade da sessao atual (a parte da
/// verificacao, QR ou codigo, chama isto depois de o utilizador confirmar).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn mark_verified(c: &st::rusqlite::Connection, cid: &str) -> Result<(), QtError> {
    let meta = st::load_session_meta(c, cid)?.ok_or_else(|| bad("contact_id"))?;
    st::kv_set(c, &format!("{KV_VERIFIED}{cid}"), &meta.peer_identity_public)?;
    Ok(())
}

/// A REGRA da ligacao direta, deste lado: contacto verificado, nao bloqueado, sem
/// "sempre pelo relay", e com motor de audio nesta build.
fn p2p_allowed(c: &st::rusqlite::Connection, cid: &str) -> Result<bool, QtError> {
    Ok(cfg!(feature = "calls") && !relay_only(c)? && !is_blocked_in(c, cid)? && is_verified(c, cid)?)
}

fn encode_addrs(addrs: &[SocketAddr]) -> Vec<u8> {
    let n = addrs.len().min(MAX_P2P_ADDRS);
    let mut out = Vec::with_capacity(1 + n * ADDR_BYTES);
    out.push(n as u8);
    for a in &addrs[..n] {
        out.extend_from_slice(&encode_addr(a));
    }
    out
}

fn decode_addrs(b: &[u8]) -> Option<Vec<SocketAddr>> {
    let n = *b.first()? as usize;
    if n == 0 || n > MAX_P2P_ADDRS || b.len() != 1 + n * ADDR_BYTES {
        return None;
    }
    b[1..].chunks_exact(ADDR_BYTES).map(decode_addr).collect()
}

/// A chamada acabou de ficar atendida (de um lado ou do outro): se a regra deixar,
/// envia "posso" e liga a ligacao direta no motor.
fn p2p_on_answered(client: &QtClient, c: &st::rusqlite::Connection, call: &mut Call) -> Result<(), QtError> {
    if call.state != State::Active || call.p2p.ready_sent || !p2p_allowed(c, &call.contact)? {
        return Ok(());
    }
    queue(client, c, &call.contact.clone(), &encode(OP_P2P_READY, &call.id, &[]), None)?;
    call.p2p.ready_sent = true;
    #[cfg(feature = "calls")]
    if let Some(a) = &call.audio {
        a.p2p_enable();
    }
    Ok(())
}

/// Envia os meus enderecos quando os dois lados disseram "posso" e o motor ja os tem.
fn p2p_send_addrs(client: &QtClient, c: &st::rusqlite::Connection, call: &mut Call) -> Result<(), QtError> {
    let p = &call.p2p;
    if call.state != State::Active || !p.ready_sent || !p.peer_ready || p.addrs_sent {
        return Ok(());
    }
    // A regra outra vez: pode ter mudado a meio (ex.: "sempre pelo relay" ligada).
    if !p2p_allowed(c, &call.contact)? {
        call.p2p.addrs_sent = true;
        return Ok(());
    }
    #[cfg(feature = "calls")]
    {
        let Some(local) = call.audio.as_ref().and_then(|a| a.p2p_local_candidates()) else { return Ok(()) };
        call.p2p.addrs_sent = true;
        if !local.is_empty() {
            queue(client, c, &call.contact.clone(), &encode(OP_P2P_ADDRS, &call.id, &encode_addrs(&local)), None)?;
        }
    }
    #[cfg(not(feature = "calls"))]
    let _ = client;
    Ok(())
}

fn derive_media(call: &Call, answer_secret: &[u8; 32]) -> Result<MediaSession, QtError> {
    let keys = CallKeys::derive(&call.offer_secret, answer_secret, &call.id, call.role);
    MediaSession::new(keys, call.pad as usize).map_err(|_| QtError::Crypto)
}

/// Tipo 11 recebido de `cid` (ja decifrado, dentro da transacao da rececao).
pub(crate) fn on_call(
    client: &QtClient,
    c: &st::rusqlite::Connection,
    cid: &str,
    body: &[u8],
    sent_at: i64,
) -> Result<ReceiveResult, QtError> {
    let no_change = ReceiveResult::NoChange { contact_id: cid.to_string() };
    if body.len() < 17 {
        return Ok(no_change);
    }
    let op = body[0];
    let call_id: [u8; 16] = body[1..17].try_into().map_err(|_| QtError::InvalidEnvelope)?;
    let rest = &body[17..];
    let mut calls = crate::client::relock(&client.calls);
    if calls.ended.contains(&call_id) {
        return Ok(no_change);
    }
    let now = now_ms();
    let is_current = calls.cur.as_ref().is_some_and(|k| k.id == call_id && k.contact == cid);

    match op {
        OP_OFFER => {
            if rest.len() != 34 || is_current {
                return Ok(no_change);
            }
            let offer_secret: [u8; 32] = rest[..32].try_into().map_err(|_| QtError::InvalidEnvelope)?;
            let pad = u16::from_be_bytes([rest[32], rest[33]]);
            if !(MIN_PAD..=MAX_PAD).contains(&(pad as usize)) {
                return Ok(no_change);
            }
            let id_hex = hex(&call_id);
            if !offer_fresh(sent_at, now) {
                calls.remember_ended(call_id);
                calls.push(CallEvent::Ended { call_id: id_hex, contact_id: cid.to_string(), reason: CallEndReason::Expired });
                return Ok(no_change);
            }
            // Ligamos os dois ao mesmo tempo um ao outro: fica a chamada com o call_id
            // menor (os dois lados decidem igual, sem trocar mensagens).
            let glare = calls.cur.as_ref().is_some_and(|k| k.state == State::Outgoing && k.contact == cid);
            if glare {
                let mine = calls.cur.as_ref().map(|k| k.id).unwrap_or_default();
                if mine < call_id {
                    calls.remember_ended(call_id);
                    return Ok(no_change);
                }
                if let Some(old) = calls.cur.take() {
                    #[cfg(feature = "calls")]
                    if let Some(a) = &old.audio {
                        a.stop();
                    }
                    if let Some(o) = old.offer_outbox {
                        hm::outbox_take(c, &o)?;
                    }
                    // A minha chamada da lugar a dele: a app troca o ecra para "a receber".
                    calls.remember_ended(old.id);
                    calls.push(CallEvent::Ended { call_id: hex(&old.id), contact_id: old.contact, reason: CallEndReason::Cancelled });
                }
            } else if calls.cur.is_some() {
                // Ja estou noutra chamada: "ocupado" e, do meu lado, chamada perdida.
                calls.remember_ended(call_id);
                queue(client, c, cid, &encode(OP_HANGUP, &call_id, &[CallEndReason::Busy.to_byte()]), None)?;
                calls.push(CallEvent::Ended { call_id: id_hex, contact_id: cid.to_string(), reason: CallEndReason::Busy });
                return Ok(no_change);
            }
            calls.cur = Some(Call {
                id: call_id,
                contact: cid.to_string(),
                state: State::Incoming,
                role: Role::Callee,
                room: RoomKeys::derive(&offer_secret, &call_id),
                offer_secret: Zeroizing::new(offer_secret),
                pad,
                since: now,
                offer_outbox: None,
                media: None,
                #[cfg(feature = "calls")]
                audio: None,
                p2p: P2p::default(),
            });
            queue(client, c, cid, &encode(OP_RINGING, &call_id, &[]), None)?;
            calls.push(CallEvent::Incoming { call_id: id_hex, contact_id: cid.to_string() });
        }
        OP_RINGING if is_current && rest.is_empty() => {
            if calls.cur.as_ref().is_some_and(|k| k.state == State::Outgoing) {
                calls.push(CallEvent::Ringing { call_id: hex(&call_id) });
            }
        }
        OP_ANSWER if is_current && rest.len() == 32 => {
            let answer: [u8; 32] = rest.try_into().map_err(|_| QtError::InvalidEnvelope)?;
            let call = calls.cur.as_mut().ok_or(QtError::Storage)?;
            if call.state != State::Outgoing {
                return Ok(no_change);
            }
            let media = derive_media(call, &answer)?;
            call.set_media(media);
            call.state = State::Active;
            call.since = now;
            call.offer_outbox = None;
            p2p_on_answered(client, c, call)?;
            calls.push(CallEvent::Answered { call_id: hex(&call_id) });
        }
        OP_P2P_READY if is_current && rest.is_empty() => {
            if let Some(call) = calls.cur.as_mut().filter(|k| k.state == State::Active) {
                call.p2p.peer_ready = true;
                p2p_send_addrs(client, c, call)?;
            }
        }
        OP_P2P_ADDRS if is_current => {
            let Some(call) = calls.cur.as_mut().filter(|k| k.state == State::Active) else { return Ok(no_change) };
            // So quem disse "posso" (e continua a cumprir a regra) aceita enderecos, uma vez.
            if !call.p2p.ready_sent || call.p2p.peer_addrs.is_some() || !p2p_allowed(c, cid)? {
                return Ok(no_change);
            }
            let Some(addrs) = decode_addrs(rest) else { return Ok(no_change) };
            call.p2p.peer_ready = true;
            #[cfg(feature = "calls")]
            if let Some(a) = &call.audio {
                a.p2p_set_peer(addrs.clone());
            }
            call.p2p.peer_addrs = Some(addrs);
            p2p_send_addrs(client, c, call)?;
        }
        OP_HANGUP if is_current && rest.len() == 1 => {
            if let Some(reason) = CallEndReason::from_byte(rest[0]) {
                calls.end_current(reason);
            }
        }
        _ => {}
    }
    Ok(no_change)
}

#[cfg(feature = "calls")]
use crate::call_audio::CallAudio;
#[cfg(feature = "calls")]
use std::sync::Arc;
#[cfg(feature = "calls")]
/// Arranca o motor para a chamada `id` (chamado por `QtClient::call_audio_start`).
pub(crate) fn start_for_call(calls: &mut Calls, id: [u8; 16], relay_url: &str, relay_public_key: &[u8]) -> Result<Arc<CallAudio>, QtError> {
    let call = match calls.cur.as_mut() {
        Some(k) if k.id == id => k,
        _ => return Err(bad("call_id")),
    };
    if let Some(a) = &call.audio {
        return Ok(a.clone());
    }
    let cfg = crate::call_audio::engine_config(relay_url)?;
    let join = qt_call::engine::join_packet(relay_public_key, call.role, &call.room).map_err(|_| bad("relay_public_key"))?;
    let engine = qt_call::engine::CallEngine::start_pending(cfg, call.room.clone(), join).map_err(|_| QtError::Crypto)?;
    let audio = CallAudio::new(engine);
    if let Some(media) = call.media.take() {
        audio.attach(media);
    }
    // Atendida antes de o motor existir: a ligacao direta ja tinha sido decidida.
    if call.p2p.ready_sent {
        audio.p2p_enable();
        if let Some(peer) = &call.p2p.peer_addrs {
            audio.p2p_set_peer(peer.clone());
        }
    }
    call.audio = Some(audio.clone());
    Ok(audio)
}

#[uniffi::export]
impl QtClient {
    /// LIGAR a um contacto (com sessao, nao bloqueado). So uma chamada de cada vez.
    /// A oferta fica na outbox (como uma mensagem: toca a campainha) e vem em `item`
    /// para enviar logo ao relay.
    pub fn call_start(&self, contact_id: String) -> Result<CallStart, QtError> {
        let c = self.lock()?;
        let mut calls = crate::client::relock(&self.calls);
        expire(self, &c, &mut calls)?;
        if calls.cur.is_some() {
            return Err(bad("call_busy"));
        }
        if is_blocked_in(&c, &contact_id)? || st::load_session(&c, &contact_id)?.is_none() {
            return Err(bad("contact_id"));
        }
        let id = random::<16>()?;
        let offer_secret = Zeroizing::new(random::<32>()?);
        let mut extra = Zeroizing::new(offer_secret.to_vec());
        extra.extend_from_slice(&DEFAULT_PAD.to_be_bytes());
        let plain = Zeroizing::new(encode(OP_OFFER, &id, &extra));
        let tx = c.unchecked_transaction()?;
        let item = queue(self, &tx, &contact_id, &plain, Some(CallWake::Call))?;
        tx.commit()?;
        let outbox_id = parse_call_id(&item.outbox_id)?;
        calls.cur = Some(Call {
            id,
            contact: contact_id,
            state: State::Outgoing,
            role: Role::Caller,
            room: RoomKeys::derive(&offer_secret, &id),
            offer_secret,
            pad: DEFAULT_PAD,
            since: now_ms(),
            offer_outbox: Some(outbox_id),
            media: None,
            #[cfg(feature = "calls")]
            audio: None,
            p2p: P2p::default(),
        });
        Ok(CallStart { call_id: hex(&id), item })
    }

    /// ATENDER a chamada que esta a tocar. Devolve a resposta (ja na outbox) para
    /// enviar logo; a partir daqui `call_seal` / `call_open` funcionam.
    pub fn call_accept(&self, call_id: String) -> Result<OutboxItem, QtError> {
        let id = parse_call_id(&call_id)?;
        let c = self.lock()?;
        let mut calls = crate::client::relock(&self.calls);
        expire(self, &c, &mut calls)?;
        let call = match calls.cur.as_mut() {
            Some(k) if k.id == id && k.state == State::Incoming => k,
            _ => return Err(bad("call_id")),
        };
        let answer = Zeroizing::new(random::<32>()?);
        let media = derive_media(call, &answer)?;
        let plain = Zeroizing::new(encode(OP_ANSWER, &id, &answer[..]));
        let tx = c.unchecked_transaction()?;
        let item = queue(self, &tx, &call.contact.clone(), &plain, None)?;
        call.set_media(media);
        call.state = State::Active;
        call.since = now_ms();
        // "Posso ligar direto" vai para a outbox DEPOIS da resposta (a app envia-a a seguir).
        p2p_on_answered(self, &tx, call)?;
        tx.commit()?;
        calls.push(CallEvent::Answered { call_id });
        Ok(item)
    }

    /// DESLIGAR, RECUSAR ou CANCELAR (o motivo sai do estado da chamada). Devolve o
    /// aviso para o outro lado, se houver (ja na outbox). Se a oferta ainda nao tinha
    /// saido do telemovel, e so retirada da outbox e nao se envia nada.
    pub fn call_hangup(&self, call_id: String) -> Result<Option<OutboxItem>, QtError> {
        let id = parse_call_id(&call_id)?;
        let c = self.lock()?;
        let mut calls = crate::client::relock(&self.calls);
        let (state, contact, offer_outbox) = match calls.cur.as_ref() {
            Some(k) if k.id == id => (k.state, k.contact.clone(), k.offer_outbox),
            _ => return Ok(None),
        };
        let reason = match state {
            State::Outgoing => CallEndReason::Cancelled,
            State::Incoming => CallEndReason::Rejected,
            State::Active => CallEndReason::Hangup,
        };
        let tx = c.unchecked_transaction()?;
        let not_sent = match offer_outbox {
            Some(o) => hm::outbox_take(&tx, &o)?.is_some(),
            None => false,
        };
        let item = if not_sent || is_blocked_in(&tx, &contact)? {
            None
        } else {
            // Cancelar antes de atender: a campainha marcada faz parar o toque do outro lado.
            let wake = (state == State::Outgoing).then_some(CallWake::CallEnd);
            Some(queue(self, &tx, &contact, &encode(OP_HANGUP, &id, &[reason.to_byte()]), wake)?)
        };
        tx.commit()?;
        calls.end_current(reason);
        Ok(item)
    }

    /// Eventos das chamadas desde a ultima chamada a esta funcao (pela ordem em que
    /// aconteceram). Chamar depois de cada lote de `receive_sealed` e de segundo a
    /// segundo enquanto houver uma chamada: e aqui que expiram as chamadas sem
    /// resposta (o aviso "sem resposta" fica na outbox para enviar).
    pub fn call_events(&self) -> Result<Vec<CallEvent>, QtError> {
        let c = self.lock()?;
        let mut calls = crate::client::relock(&self.calls);
        expire(self, &c, &mut calls)?;
        // Os meus enderecos ficam prontos uns instantes depois de atender: envia-os aqui.
        if let Some(call) = calls.cur.as_mut() {
            let tx = c.unchecked_transaction()?;
            p2p_send_addrs(self, &tx, call)?;
            tx.commit()?;
        }
        Ok(calls.events.drain(..).collect())
    }

    /// Ha uma chamada a decorrer (a tocar ou atendida)? Devolve o call_id.
    pub fn call_current(&self) -> Result<Option<String>, QtError> {
        let calls = crate::client::relock(&self.calls);
        Ok(calls.cur.as_ref().map(|k| hex(&k.id)))
    }

    /// "Chamadas sempre pelo relay" (definicao deste telemovel). Ligada: nenhuma chamada
    /// liga direto, mesmo com contactos verificados (o outro lado tambem fica no relay).
    pub fn calls_relay_only(&self) -> Result<bool, QtError> {
        let c = self.lock()?;
        relay_only(&c)
    }

    /// Liga ou desliga "chamadas sempre pelo relay". Ligada a meio de uma chamada: o
    /// audio volta ao relay e os meus enderecos ja nao saem.
    pub fn set_calls_relay_only(&self, on: bool) -> Result<(), QtError> {
        let c = self.lock()?;
        st::kv_set(&c, KV_RELAY_ONLY, &[on as u8])?;
        #[cfg(feature = "calls")]
        if on {
            let calls = crate::client::relock(&self.calls);
            if let Some(a) = calls.cur.as_ref().and_then(|k| k.audio.as_ref()) {
                a.p2p_disable();
            }
        }
        Ok(())
    }

    /// Pedido para ENTRAR NA SALA do relay de chamadas (UDP 443 ou, se o UDP estiver
    /// bloqueado, a ligacao `wss://.../call`). Cifrado para a chave estatica do relay
    /// (`relay_public_key`, a do pin): o relay fica a saber a sala, nunca a conta. Pode
    /// ser usado logo ao ligar e enquanto toca. Repetir a cada ~500 ms ate chegar a
    /// resposta (`call_join_ok`) e depois a cada 10 s enquanto nao houver audio.
    pub fn call_join_packet(&self, call_id: String, relay_public_key: Vec<u8>) -> Result<Vec<u8>, QtError> {
        let id = parse_call_id(&call_id)?;
        if relay_public_key.len() != 32 {
            return Err(bad("relay_public_key"));
        }
        let calls = crate::client::relock(&self.calls);
        let call = match calls.cur.as_ref() {
            Some(k) if k.id == id => k,
            _ => return Err(bad("call_id")),
        };
        let payload = Zeroizing::new(join_payload(call.role, &call.room));
        let msg = qt_transport::noise_n_seal(&relay_public_key, JOIN_PROLOGUE, &payload[..]).map_err(|_| QtError::Crypto)?;
        let mut out = Vec::with_capacity(1 + msg.len());
        out.push(JOIN_KIND);
        out.extend_from_slice(&msg);
        Ok(out)
    }

    /// Resposta do relay ao pedido de entrada. `None`: nao e a resposta desta sala
    /// (ignorar). `Some(true)`: o outro telemovel ja esta na sala.
    pub fn call_join_ok(&self, call_id: String, packet: Vec<u8>) -> Result<Option<bool>, QtError> {
        let id = parse_call_id(&call_id)?;
        let calls = crate::client::relock(&self.calls);
        match calls.cur.as_ref() {
            Some(k) if k.id == id => Ok(parse_join_ok(&k.room, &packet)),
            _ => Err(bad("call_id")),
        }
    }

    /// Cifra um frame de audio (pode ser vazio, para abrir a sala). Todos os pacotes
    /// da chamada tem o mesmo tamanho. So depois de atendida.
    pub fn call_seal(&self, call_id: String, frame: Vec<u8>) -> Result<Vec<u8>, QtError> {
        let id = parse_call_id(&call_id)?;
        let mut calls = crate::client::relock(&self.calls);
        match calls.cur.as_mut() {
            Some(Call { id: cur, media: Some(m), .. }) if *cur == id => m.seal(&frame).map_err(|_| bad("frame")),
            _ => Err(bad("call_id")),
        }
    }

    /// Decifra um pacote de audio do outro lado. `None`: pacote invalido, repetido ou
    /// de outra chamada (descartar sem mais nada).
    pub fn call_open(&self, call_id: String, packet: Vec<u8>) -> Result<Option<MediaFrame>, QtError> {
        let id = parse_call_id(&call_id)?;
        let mut calls = crate::client::relock(&self.calls);
        match calls.cur.as_mut() {
            Some(Call { id: cur, media: Some(m), .. }) if *cur == id => {
                Ok(m.open(&packet).ok().map(|(counter, frame)| MediaFrame { counter, frame }))
            }
            _ => Err(bad("call_id")),
        }
    }
}

/// Chamadas a tocar ha demasiado tempo: quem liga desiste (e avisa); quem recebe para
/// de tocar (chamada perdida).
fn expire(client: &QtClient, c: &st::rusqlite::Connection, calls: &mut Calls) -> Result<(), QtError> {
    let Some(call) = calls.cur.as_ref() else { return Ok(()) };
    if call.state == State::Active || now_ms() - call.since < RING_TIMEOUT_MS {
        return Ok(());
    }
    let (id, contact, state) = (call.id, call.contact.clone(), call.state);
    if state == State::Outgoing && !is_blocked_in(c, &contact)? {
        let tx = c.unchecked_transaction()?;
        queue(client, &tx, &contact, &encode(OP_HANGUP, &id, &[CallEndReason::NoAnswer.to_byte()]), Some(CallWake::CallEnd))?;
        tx.commit()?;
    }
    let reason = if state == State::Outgoing { CallEndReason::NoAnswer } else { CallEndReason::Cancelled };
    calls.end_current(reason);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oferta_atrasada_nao_toca() {
        let now = 1_000_000;
        assert!(offer_fresh(now - 1_000, now));
        assert!(!offer_fresh(now - OFFER_TTL_MS, now));
        // Relogio do outro lado adiantado: conta como enviada agora.
        assert!(offer_fresh(now + 600_000, now));
    }

    #[test]
    fn motivos_ida_e_volta() {
        for r in [
            CallEndReason::Hangup,
            CallEndReason::Rejected,
            CallEndReason::Busy,
            CallEndReason::NoAnswer,
            CallEndReason::Cancelled,
            CallEndReason::Expired,
        ] {
            assert_eq!(CallEndReason::from_byte(r.to_byte()), Some(r));
        }
        assert_eq!(CallEndReason::from_byte(0), None);
    }

    #[test]
    fn formato_dos_enderecos() {
        let a: Vec<SocketAddr> = vec!["85.1.2.3:40000".parse().unwrap(), "[2a01::5]:40000".parse().unwrap()];
        let b = encode_addrs(&a);
        assert_eq!(b.len(), 1 + 2 * ADDR_BYTES);
        assert_eq!(decode_addrs(&b), Some(a));
        assert_eq!(decode_addrs(&[0]), None, "lista vazia");
        assert_eq!(decode_addrs(&b[..b.len() - 1]), None, "cortada");
        let mut many = vec![6u8];
        many.extend(std::iter::repeat_n(encode_addr(&"1.2.3.4:5".parse().unwrap()), 6).flatten());
        assert_eq!(decode_addrs(&many), None, "mais de 5");
        // A mensagem inteira cabe no limite do tipo 11 (senao o outro lado ignorava-a).
        let full = encode(OP_P2P_ADDRS, &[1; 16], &encode_addrs(&vec!["1.2.3.4:5".parse().unwrap(); 9]));
        assert!(full.len() - HDR <= 128, "{}", full.len() - HDR);
        let mut bad = b.clone();
        bad[1] = 7;
        assert_eq!(decode_addrs(&bad), None, "familia desconhecida");
    }

    #[test]
    fn formato_da_oferta() {
        let id = [7u8; 16];
        let p = encode(OP_OFFER, &id, &[1u8; 34]);
        assert_eq!((p[0], p[1], &p[2..18]), (PAYLOAD_V1, T_CALL, &id[..]));
        assert_eq!((p[HDR], &p[HDR + 1..HDR + 17], p.len()), (OP_OFFER, &id[..], HDR + 17 + 34));
    }
}

/// A regra da ligacao direta com dois telemoveis reais (QtClient + motor de audio), sem
/// relay: o audio so pode passar pelo caminho direto (127.0.0.1).
#[cfg(all(test, feature = "calls"))]
mod p2p_tests {
    use super::*;
    use crate::client::{peer_bundle_from_relay_bytes, PublicIdentity};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    struct Peer {
        c: Arc<QtClient>,
        id: PublicIdentity,
    }

    impl Peer {
        fn new(name: &str, key: u8) -> Peer {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let path = format!("/tmp/qt_p2p_{name}_{}_{n}.sqlite", std::process::id());
            let _ = std::fs::remove_file(&path);
            let c = QtClient::open(path, vec![key; 32]).unwrap();
            let id = c.create_identity().unwrap();
            Peer { c, id }
        }
        fn cid(&self) -> String {
            hex(&self.id.identity_public)
        }
        /// (enviei "posso", o outro disse "posso", enviei enderecos, recebi enderecos)
        fn state(&self) -> (bool, bool, bool, bool) {
            let calls = crate::client::relock(&self.c.calls);
            let p = &calls.cur.as_ref().expect("chamada").p2p;
            (p.ready_sent, p.peer_ready, p.addrs_sent, p.peer_addrs.is_some())
        }
        fn verify(&self, other: &Peer) {
            mark_verified(&self.c.lock().unwrap(), &other.cid()).unwrap();
        }
    }

    /// Entrega tudo o que `from` tem na outbox para `to`. Devolve quantas mensagens.
    fn deliver(from: &Peer, to: &Peer) -> usize {
        let items = from.c.outbox().unwrap();
        for item in &items {
            if item.contact_id == to.cid() {
                to.c.receive_sealed(item.blob.clone()).unwrap();
            }
            from.c.outbox_sent(item.outbox_id.clone()).unwrap();
        }
        items.len()
    }

    fn pair() -> (Peer, Peer) {
        let (a, b) = (Peer::new("a", 0xA1), Peer::new("b", 0xB2));
        let bundle = peer_bundle_from_relay_bytes(b.c.publishable_bundle().unwrap()).unwrap();
        a.c.start_session(b.id.public_id.clone(), bundle).unwrap();
        a.c.send_text(b.cid(), "ola".into()).unwrap();
        deliver(&a, &b);
        b.c.send_text(a.cid(), "ola!".into()).unwrap();
        deliver(&b, &a);
        (a, b)
    }

    struct Call {
        id: String,
        audio_a: Arc<crate::call_audio::CallAudio>,
        audio_b: Arc<crate::call_audio::CallAudio>,
    }

    /// A liga, B atende. Verifica que ANTES de atender nada de "posso" nem enderecos sai, e
    /// devolve quantas mensagens B enviou ao atender.
    fn call(a: &Peer, b: &Peer) -> (Call, usize) {
        let url = "ws://127.0.0.1:9".to_string(); // sem relay: so o caminho direto funciona
        let pin = qt_transport::generate_static_keypair().unwrap().public;
        let id = a.c.call_start(b.cid()).unwrap().call_id;
        let audio_a = a.c.call_audio_start(id.clone(), url.clone(), pin.clone()).unwrap();
        assert_eq!(deliver(a, b), 1, "so a oferta");
        assert!(matches!(b.c.call_events().unwrap()[0], CallEvent::Incoming { .. }));
        let audio_b = b.c.call_audio_start(id.clone(), url, pin).unwrap();
        // A tocar: nenhum lado disse "posso" nem tem enderecos para dar.
        std::thread::sleep(Duration::from_millis(1700));
        a.c.call_events().unwrap();
        b.c.call_events().unwrap();
        assert_eq!(deliver(b, a), 1, "so o \"a tocar\"");
        assert_eq!(a.state(), (false, false, false, false));
        assert_eq!(b.state(), (false, false, false, false));
        assert!(audio_a.p2p_local_candidates().is_none() && audio_b.p2p_local_candidates().is_none());
        b.c.call_accept(id.clone()).unwrap();
        let sent_on_accept = deliver(b, a);
        (Call { id, audio_a, audio_b }, sent_on_accept)
    }

    /// Faz o que a app faz a cada segundo (eventos + outbox) durante `secs` ou ate `done`.
    fn run(a: &Peer, b: &Peer, secs: u64, mut done: impl FnMut() -> bool) -> bool {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(secs) {
            a.c.call_events().unwrap();
            b.c.call_events().unwrap();
            deliver(a, b);
            deliver(b, a);
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    #[test]
    fn verificados_dos_dois_lados_ligam_direto() {
        let (a, b) = pair();
        a.verify(&b);
        b.verify(&a);
        let (k, on_accept) = call(&a, &b);
        assert_eq!(on_accept, 2, "a resposta e o \"posso\"");
        let direct = run(&a, &b, 10, || k.audio_a.stats().direct && k.audio_b.stats().direct);
        assert!(direct, "A {:?} B {:?}", k.audio_a.stats(), k.audio_b.stats());
        assert_eq!(a.state(), (true, true, true, true));
        assert_eq!(b.state(), (true, true, true, true));
        // "Sempre pelo relay" ligada a meio: o audio deixa logo o caminho direto.
        a.c.set_calls_relay_only(true).unwrap();
        assert!(a.c.calls_relay_only().unwrap());
        assert!(run(&a, &b, 2, || !k.audio_a.stats().direct));
        a.c.call_hangup(k.id).unwrap();
    }

    /// O outro lado nao me verificou (ou e uma app antiga, que nunca envia "posso" e ignora
    /// as ops 5 e 6): nenhum dos dois envia enderecos.
    #[test]
    fn so_um_lado_verificou_nenhum_endereco_sai() {
        let (a, b) = pair();
        a.verify(&b);
        let (k, on_accept) = call(&a, &b);
        assert_eq!(on_accept, 1, "B so envia a resposta");
        run(&a, &b, 4, || false);
        assert_eq!(a.state(), (true, false, false, false), "A disse posso, mais nada");
        assert_eq!(b.state(), (false, true, false, false), "B ouviu, nao respondeu nem enviou enderecos");
        assert!(!k.audio_a.stats().direct && !k.audio_b.stats().direct);
        assert!(k.audio_b.p2p_local_candidates().is_none(), "B nem recolheu enderecos");
    }

    #[test]
    fn sempre_pelo_relay_num_lado_fica_tudo_no_relay() {
        let (a, b) = pair();
        a.verify(&b);
        b.verify(&a);
        b.c.set_calls_relay_only(true).unwrap();
        let (k, on_accept) = call(&a, &b);
        assert_eq!(on_accept, 1);
        run(&a, &b, 4, || false);
        assert_eq!(a.state(), (true, false, false, false));
        assert_eq!(b.state(), (false, true, false, false));
        assert!(!k.audio_a.stats().direct && !k.audio_b.stats().direct);
    }

    #[test]
    fn verificacao_cai_se_a_chave_mudar() {
        let (a, b) = pair();
        let c = a.c.lock().unwrap();
        assert!(!is_verified(&c, &b.cid()).unwrap());
        mark_verified(&c, &b.cid()).unwrap();
        assert!(is_verified(&c, &b.cid()).unwrap());
        // A chave guardada deixa de ser a da sessao (o contacto mudou de chave).
        st::kv_set(&c, &format!("{KV_VERIFIED}{}", b.cid()), &[7u8; 32]).unwrap();
        assert!(!is_verified(&c, &b.cid()).unwrap());
        assert!(!p2p_allowed(&c, &b.cid()).unwrap());
    }
}
