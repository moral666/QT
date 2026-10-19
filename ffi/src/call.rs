//! Chamadas de voz 1 a 1: sinalizacao dentro do Double Ratchet (tipo 11).
//!
//! DESENHO (aprovado pelo Nuno a 19/10/2026, ver docs/protocol-spec.md, seccao 18):
//!   * A oferta, o "a tocar", a resposta e o desligar vao como mensagens cifradas
//!     iguais as outras (envelope v3 com enchimento): o relay nao sabe que e uma
//!     chamada. A oferta toca a campainha como uma mensagem; o resto vai `silent`.
//!   * Cada lado contribui 32 bytes aleatorios (oferta e resposta). As chaves do
//!     audio, o codigo da sala e a chave do relay saem dai (`qt_core::call`).
//!   * Tudo fica SO em memoria (nada em disco). Uma chamada de cada vez.
//!   * O audio (fases seguintes) passa sempre pelo relay; nunca ha ligacao direta.
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

use std::collections::VecDeque;

use qt_core::call::{CallKeys, MediaSession, Role, MAX_PAD, MIN_PAD};
use qt_storage as st;
use qt_storage::messages as hm;
use zeroize::Zeroizing;

use crate::client::{bad, hex, unhex, QtClient, QtError};
use crate::history::{is_blocked_in, now_ms, OutboxItem, ReceiveResult, HDR, PAYLOAD_V1};

pub(crate) const T_CALL: u8 = 11;
const OP_OFFER: u8 = 1;
const OP_ANSWER: u8 = 2;
const OP_HANGUP: u8 = 3;
const OP_RINGING: u8 = 4;

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
    pad: u16,
    since: i64,
    /// So para quem liga: o id da oferta na outbox (para a tirar se cancelar antes de sair).
    offer_outbox: Option<[u8; 16]>,
    media: Option<MediaSession>,
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

/// Cifra uma mensagem de chamada para `cid` e poe-na na outbox.
fn queue(client: &QtClient, c: &st::rusqlite::Connection, cid: &str, plain: &[u8], ring: bool) -> Result<OutboxItem, QtError> {
    let id = random::<16>()?;
    let blob = client.seal_with(c, cid, plain)?;
    let kind = if ring { hm::OUTBOX_MESSAGE } else { hm::OUTBOX_RECEIPT };
    hm::outbox_put(c, &id, cid, kind, &blob)?;
    Ok(OutboxItem { outbox_id: hex(&id), contact_id: cid.to_string(), blob, silent: !ring, relay_id: None })
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
    let mut calls = client.calls.lock().map_err(|_| QtError::Storage)?;
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
                queue(client, c, cid, &encode(OP_HANGUP, &call_id, &[CallEndReason::Busy.to_byte()]), false)?;
                calls.push(CallEvent::Ended { call_id: id_hex, contact_id: cid.to_string(), reason: CallEndReason::Busy });
                return Ok(no_change);
            }
            calls.cur = Some(Call {
                id: call_id,
                contact: cid.to_string(),
                state: State::Incoming,
                role: Role::Callee,
                offer_secret: Zeroizing::new(offer_secret),
                pad,
                since: now,
                offer_outbox: None,
                media: None,
            });
            queue(client, c, cid, &encode(OP_RINGING, &call_id, &[]), false)?;
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
            call.media = Some(derive_media(call, &answer)?);
            call.state = State::Active;
            call.since = now;
            call.offer_outbox = None;
            calls.push(CallEvent::Answered { call_id: hex(&call_id) });
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

#[uniffi::export]
impl QtClient {
    /// LIGAR a um contacto (com sessao, nao bloqueado). So uma chamada de cada vez.
    /// A oferta fica na outbox (como uma mensagem: toca a campainha) e vem em `item`
    /// para enviar logo ao relay.
    pub fn call_start(&self, contact_id: String) -> Result<CallStart, QtError> {
        let c = self.lock()?;
        let mut calls = self.calls.lock().map_err(|_| QtError::Storage)?;
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
        let item = queue(self, &tx, &contact_id, &plain, true)?;
        tx.commit()?;
        let outbox_id = parse_call_id(&item.outbox_id)?;
        calls.cur = Some(Call {
            id,
            contact: contact_id,
            state: State::Outgoing,
            role: Role::Caller,
            offer_secret,
            pad: DEFAULT_PAD,
            since: now_ms(),
            offer_outbox: Some(outbox_id),
            media: None,
        });
        Ok(CallStart { call_id: hex(&id), item })
    }

    /// ATENDER a chamada que esta a tocar. Devolve a resposta (ja na outbox) para
    /// enviar logo; a partir daqui `call_seal` / `call_open` funcionam.
    pub fn call_accept(&self, call_id: String) -> Result<OutboxItem, QtError> {
        let id = parse_call_id(&call_id)?;
        let c = self.lock()?;
        let mut calls = self.calls.lock().map_err(|_| QtError::Storage)?;
        expire(self, &c, &mut calls)?;
        let call = match calls.cur.as_mut() {
            Some(k) if k.id == id && k.state == State::Incoming => k,
            _ => return Err(bad("call_id")),
        };
        let answer = Zeroizing::new(random::<32>()?);
        let media = derive_media(call, &answer)?;
        let plain = Zeroizing::new(encode(OP_ANSWER, &id, &answer[..]));
        let tx = c.unchecked_transaction()?;
        let item = queue(self, &tx, &call.contact.clone(), &plain, false)?;
        tx.commit()?;
        call.media = Some(media);
        call.state = State::Active;
        call.since = now_ms();
        calls.push(CallEvent::Answered { call_id });
        Ok(item)
    }

    /// DESLIGAR, RECUSAR ou CANCELAR (o motivo sai do estado da chamada). Devolve o
    /// aviso para o outro lado, se houver (ja na outbox). Se a oferta ainda nao tinha
    /// saido do telemovel, e so retirada da outbox e nao se envia nada.
    pub fn call_hangup(&self, call_id: String) -> Result<Option<OutboxItem>, QtError> {
        let id = parse_call_id(&call_id)?;
        let c = self.lock()?;
        let mut calls = self.calls.lock().map_err(|_| QtError::Storage)?;
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
            Some(queue(self, &tx, &contact, &encode(OP_HANGUP, &id, &[reason.to_byte()]), false)?)
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
        let mut calls = self.calls.lock().map_err(|_| QtError::Storage)?;
        expire(self, &c, &mut calls)?;
        Ok(calls.events.drain(..).collect())
    }

    /// Ha uma chamada a decorrer (a tocar ou atendida)? Devolve o call_id.
    pub fn call_current(&self) -> Result<Option<String>, QtError> {
        let calls = self.calls.lock().map_err(|_| QtError::Storage)?;
        Ok(calls.cur.as_ref().map(|k| hex(&k.id)))
    }

    /// Cifra um frame de audio (pode ser vazio, para abrir a sala). Todos os pacotes
    /// da chamada tem o mesmo tamanho. So depois de atendida.
    pub fn call_seal(&self, call_id: String, frame: Vec<u8>) -> Result<Vec<u8>, QtError> {
        let id = parse_call_id(&call_id)?;
        let mut calls = self.calls.lock().map_err(|_| QtError::Storage)?;
        match calls.cur.as_mut() {
            Some(Call { id: cur, media: Some(m), .. }) if *cur == id => m.seal(&frame).map_err(|_| bad("frame")),
            _ => Err(bad("call_id")),
        }
    }

    /// Decifra um pacote de audio do outro lado. `None`: pacote invalido, repetido ou
    /// de outra chamada (descartar sem mais nada).
    pub fn call_open(&self, call_id: String, packet: Vec<u8>) -> Result<Option<MediaFrame>, QtError> {
        let id = parse_call_id(&call_id)?;
        let mut calls = self.calls.lock().map_err(|_| QtError::Storage)?;
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
        queue(client, &tx, &contact, &encode(OP_HANGUP, &id, &[CallEndReason::NoAnswer.to_byte()]), false)?;
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
    fn formato_da_oferta() {
        let id = [7u8; 16];
        let p = encode(OP_OFFER, &id, &[1u8; 34]);
        assert_eq!((p[0], p[1], &p[2..18]), (PAYLOAD_V1, T_CALL, &id[..]));
        assert_eq!((p[HDR], &p[HDR + 1..HDR + 17], p.len()), (OP_OFFER, &id[..], HDR + 17 + 34));
    }
}
