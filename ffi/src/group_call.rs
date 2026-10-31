//! Chamadas de GRUPO (0.9.62): ate 5 pessoas (eu e mais 4), voz e video.
//!
//! DESENHO (aprovado pelo Nuno a 30/10/2026):
//!   * Uma chamada de grupo e uma MALHA de chamadas a dois ("pernas"): cada par de
//!     participantes tem a sua perna, com chaves proprias (`qt_core::call`, da oferta E da
//!     resposta da perna) e o seu motor (`qt_call::engine`, o mesmo das chamadas a dois). O
//!     relay das chamadas nao muda: cada perna e uma sala de 2.
//!   * A voz de cada perna tenta a LIGACAO DIRETA e, se nao der, vai pelo relay (AWS). O VIDEO
//!     so vai pela ligacao direta (nunca pelo relay): um par sem direto ve so a foto.
//!   * A ligacao direta e com TODOS os participantes, mesmo os que nao sao meus contactos: quem
//!     entra numa chamada de grupo mostra o IP aos outros participantes. Por isso, antes da
//!     primeira chamada de grupo, a app mostra "Nas chamadas de grupo, o teu endereco de rede
//!     fica visivel aos outros participantes" com "Aceitar" (`group_call_accept_notice`); sem
//!     isso nao se liga nem se entra. Nada disto aparece durante a chamada.
//!   * Entrar numa chamada de grupo com video e aceitar ver o video dos outros: cada um decide
//!     so se mostra a propria camara.
//!   * So entre membros do grupo, nunca com bloqueados. No maximo 5 na chamada.
//!   * Tudo so em memoria. Uma chamada de cada vez (de grupo OU a dois).
//!
//! QUEM OFERECE CADA PERNA: o participante com o id (hex) MENOR. Quem entra conhece quem ja
//! esta (pelo convite e pelos "entrei" que recebeu); quem ja esta fica a saber de quem entra
//! pelo "entrei" dele. Assim cada par tem exatamente um que oferece, seja qual for a ordem.
//!
//! FORMATO: payload v1, tipo 14; `msg_id` = id da chamada (uma app antiga mostra "atualiza a
//! app" uma so vez por chamada). Corpo: [1 op][16 id da chamada] e:
//!   op 1 convite:  [16 group_id][1 bit0 = videochamada]  (a todos os membros; toca)
//!   op 2 entrei:   [16 group_id]                          (a todos os membros)
//!   op 3 sai:      [16 group_id]                          (a todos os membros)
//!   op 4 oferta da perna:   [16 leg_id][32 offer_secret][2 pad BE] e, opcional, [1 n][n nome
//!                  do relay das chamadas de quem oferece] (federacao, como no tipo 11)
//!   op 5 resposta da perna: [16 leg_id][32 answer_secret]
//!   op 6 enderecos: [16 leg_id][1 n, 1..=4][n x 19 endereco]
//!   op 7 camara:   [1 bit0 = camara ligada]               (a cada perna)

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use qt_core::call::{decode_addr, encode_addr, CallKeys, MediaSession, Role, RoomKeys, ADDR_BYTES, MAX_PAD, MIN_PAD};
use qt_storage as st;
use zeroize::Zeroizing;

use crate::client::{bad, hex, unhex, QtClient, QtError};
use crate::history::{is_blocked_in, now_ms, CallWake, OutboxItem, ReceiveResult, HDR, PAYLOAD_V1};

pub(crate) const T_GROUP_CALL: u8 = 14;
const OP_INVITE: u8 = 1;
const OP_JOINED: u8 = 2;
const OP_LEFT: u8 = 3;
const OP_LEG_OFFER: u8 = 4;
const OP_LEG_ANSWER: u8 = 5;
const OP_ADDRS: u8 = 6;
const OP_CAMERA: u8 = 7;
/// Nome desta chamada (0.9.67): `[1 n][n UTF-8]`, o que escolhi para os outros me chamarem.
const OP_NAME: u8 = 8;
/// Nome maximo (bytes UTF-8).
pub const MAX_CALL_NAME: usize = 32;

/// Eu e mais 4.
pub const MAX_GROUP_CALL: usize = 5;
/// Enderecos por mensagem (o corpo do tipo 14 tem no maximo 192 bytes).
const MAX_ADDRS: usize = 4;
const _: () = assert!(17 + 16 + 1 + MAX_ADDRS * ADDR_BYTES <= 192);
/// Corpo maximo do tipo 14 (`history.rs`).
pub(crate) const MAX_BODY: usize = 192;
/// O aviso do IP foi aceite.
const KV_NOTICE: &str = "gcall:notice";
const MAX_EVENTS: usize = 64;
const MAX_ENDED: usize = 16;
/// Quem saiu sem avisar (telemovel sem rede ou sem bateria, app fechada a forca): uma perna sem
/// nada do outro lado ha tanto tempo (ou oferecida e sem resposta), ou alguem que disse "entrei"
/// e a perna nunca chegou, sai da chamada como se tivesse enviado "sai". Antes ficava la ate ao
/// fim (ocupava um dos 5 lugares e o meu audio continuava a ir para ele, pelo relay).
#[cfg(feature = "calls")]
const LEG_SILENT_MS: i64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GroupCallEndReason {
    /// Sai eu.
    Left,
    /// Toquei e ninguem atendeu a tempo, ou toda a gente saiu antes de eu entrar.
    Missed,
    /// Ja estava noutra chamada quando me convidaram.
    Busy,
}

/// O que aconteceu as chamadas de grupo desde a ultima vez (`group_call_events`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum GroupCallEvent {
    /// Convidaram-me: tocar ("Chamada de grupo: [grupo]").
    Incoming { call_id: String, group_id: String, from: String, video: bool },
    /// Mudou quem esta na chamada ou o estado de alguem (ligado, camara): atualizar o ecra.
    Changed { call_id: String },
    /// Acabou para mim.
    Ended { call_id: String, group_id: String, reason: GroupCallEndReason },
    /// Mudou a chamada a decorrer num grupo em que nao estou (`group_call_ongoing`).
    Ongoing { group_id: String },
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct GroupCallParticipant {
    pub contact_id: String,
    /// A perna com esta pessoa ja tem chaves (ouvimo-nos).
    pub connected: bool,
    /// A camara dela esta ligada.
    pub camera: bool,
    /// O nome que essa pessoa escolheu para esta chamada (0.9.67), se disse algum.
    pub name: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct GroupCallStart {
    pub call_id: String,
    /// Os convites, ja na outbox: enviar ja ao relay (tocam a campainha).
    pub items: Vec<OutboxItem>,
}

/// Um relay das chamadas da federacao (nome, URL do relay das mensagens e o pin).
#[derive(Debug, Clone, uniffi::Record)]
pub struct CallRelayInfo {
    pub name: String,
    pub url: String,
    pub public_key: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Convidaram-me; ainda nao entrei.
    Ringing,
    /// Estou na chamada.
    Joined,
}

/// A perna com um participante.
struct Leg {
    id: [u8; 16],
    role: Role,
    offer_secret: Zeroizing<[u8; 32]>,
    room: RoomKeys,
    pad: u16,
    /// Relay das chamadas desta perna (o de quem ofereceu). `None`: o principal.
    relay: Option<String>,
    /// Chaves do audio (com a resposta), ate o motor existir.
    media: Option<MediaSession>,
    answered: bool,
    addrs_sent: bool,
    peer_addrs: Option<Vec<SocketAddr>>,
    peer_camera: bool,
    peer_name: Option<String>,
    #[cfg(feature = "calls")]
    audio: Option<Arc<crate::call_audio::CallAudio>>,
    /// Pacotes recebidos dele (`CallAudio::heard`) e quando esse numero mudou pela ultima vez.
    #[cfg(feature = "calls")]
    heard: u64,
    #[cfg(feature = "calls")]
    heard_at: i64,
}

struct GroupCall {
    id: [u8; 16],
    group: String,
    state: State,
    video: bool,
    since: i64,
    /// Outros que ja estao na chamada (pelo convite e pelos "entrei").
    present: BTreeSet<String>,
    legs: BTreeMap<String, Leg>,
    camera: bool,
    /// O meu nome nesta chamada (vai a cada perna quando tem chaves).
    my_name: Option<String>,
    /// Estao na chamada mas ainda sem perna comigo (era a vez deles de oferecer): desde quando.
    waiting: BTreeMap<String, i64>,
}

#[derive(Default)]
pub(crate) struct GroupCalls {
    cur: Option<GroupCall>,
    events: VecDeque<GroupCallEvent>,
    ended: VecDeque<[u8; 16]>,
    /// Chamadas de grupo a decorrer em que NAO estou (recusei ou nao atendi a tempo): da para
    /// entrar depois, enquanto houver alguem (0.9.65). id -> (grupo, quem esta, video, ultima noticia).
    known: BTreeMap<[u8; 16], (String, BTreeSet<String>, bool, i64)>,
    #[cfg(feature = "calls")]
    audio: Option<Arc<GroupAudio>>,
    /// So nos testes: outro prazo para `LEG_SILENT_MS`.
    #[cfg(all(test, feature = "calls"))]
    leg_timeout: Option<i64>,
}

impl GroupCalls {
    pub(crate) fn in_call(&self) -> bool {
        self.cur.as_ref().is_some_and(|g| g.state == State::Joined)
    }

    /// Ha uma chamada de grupo (a tocar ou dentro)? Um toque que ja passou do prazo acaba aqui
    /// (chamada perdida): com a app em segundo plano ninguem chama `group_call_events`, e um
    /// convite recebido assim deixava o telemovel "ocupado" para as chamadas a dois ate o ecra
    /// abrir.
    pub(crate) fn cur_any(&mut self) -> bool {
        expire(self);
        self.cur.is_some()
    }

    /// O nucleo vai fechar: o audio e o video do grupo param (ver `Drop for QtClient`).
    #[cfg(feature = "calls")]
    pub(crate) fn stop_media(&self) {
        if let Some(a) = &self.audio {
            a.stop();
        }
        if let Some(g) = &self.cur {
            for leg in g.legs.values() {
                leg.stop();
            }
        }
    }

    fn push(&mut self, e: GroupCallEvent) {
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

    fn end(&mut self, reason: GroupCallEndReason) {
        if let Some(g) = self.cur.take() {
            #[cfg(feature = "calls")]
            if let Some(a) = self.audio.take() {
                a.stop();
            }
            // Recusei (ou nao atendi) e ainda ha gente na chamada: fica para poder entrar depois.
            if g.state == State::Ringing && !g.present.is_empty() {
                self.known.insert(g.id, (g.group.clone(), g.present.clone(), g.video, now_ms()));
            } else {
                self.remember_ended(g.id);
            }
            self.push(GroupCallEvent::Ended { call_id: hex(&g.id), group_id: g.group, reason });
        }
    }

    /// Esquece as chamadas sem noticias ha mais de 3 h (alguem saiu sem avisar).
    fn prune_known(&mut self) {
        let now = now_ms();
        self.known.retain(|_, k| now - k.3 < 3 * 3600 * 1000);
    }

    /// Uma chamada do grupo `gid` em que nao estou mudou (a conversa do grupo mostra ou tira
    /// o "Entrar na chamada").
    fn changed_known(&mut self, gid: &str) {
        let e = GroupCallEvent::Ongoing { group_id: gid.to_string() };
        if self.events.back() != Some(&e) {
            self.push(e);
        }
    }

    fn changed(&mut self) {
        if let Some(g) = &self.cur {
            let e = GroupCallEvent::Changed { call_id: hex(&g.id) };
            if self.events.back() != Some(&e) {
                self.push(e);
            }
        }
    }
}

fn random<const N: usize>() -> Result<[u8; N], QtError> {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).map_err(|_| QtError::Crypto)?;
    Ok(b)
}

fn parse_id(s: &str) -> Result<[u8; 16], QtError> {
    unhex(s).and_then(|b| b.try_into().ok()).ok_or_else(|| bad("call_id"))
}

fn encode(op: u8, id: &[u8; 16], extra: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HDR + 17 + extra.len());
    out.push(PAYLOAD_V1);
    out.push(T_GROUP_CALL);
    out.extend_from_slice(id);
    out.extend_from_slice(&now_ms().to_be_bytes());
    out.push(op);
    out.extend_from_slice(id);
    out.extend_from_slice(extra);
    out
}

fn queue(client: &QtClient, c: &st::rusqlite::Connection, cid: &str, plain: &[u8], wake: Option<CallWake>) -> Result<OutboxItem, QtError> {
    crate::call::queue(client, c, cid, plain, wake)
}

/// Envia a cada um (com sessao e nao bloqueado). Devolve o que ficou na outbox.
fn broadcast(client: &QtClient, c: &st::rusqlite::Connection, to: &[String], plain: &[u8], wake: Option<CallWake>) -> Result<Vec<OutboxItem>, QtError> {
    let mut items = Vec::new();
    for cid in to {
        if is_blocked_in(c, cid)? || st::load_session(c, cid)?.is_none() {
            continue;
        }
        items.push(queue(client, c, cid, plain, wake)?);
    }
    Ok(items)
}

fn notice_accepted(c: &st::rusqlite::Connection) -> Result<bool, QtError> {
    Ok(st::kv_get(c, KV_NOTICE)?.is_some())
}

/// Um nome de chamada valido: sem espacos a volta, sem quebras nem controlos, 1 a 32 bytes
/// (cortado numa letra inteira). Vazio: nenhum.
pub(crate) fn clean_name(n: &str) -> Option<String> {
    let t: String = n.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string();
    if t.is_empty() {
        return None;
    }
    let mut end = t.len().min(MAX_CALL_NAME);
    while !t.is_char_boundary(end) {
        end -= 1;
    }
    Some(t[..end].trim_end().to_string())
}

fn name_body(n: &str) -> Vec<u8> {
    let mut b = vec![n.len() as u8];
    b.extend_from_slice(n.as_bytes());
    b
}

fn parse_name(b: &[u8]) -> Option<String> {
    let (&n, rest) = b.split_first()?;
    if n as usize != rest.len() {
        return None;
    }
    clean_name(std::str::from_utf8(rest).ok()?)
}

fn encode_addrs(addrs: &[SocketAddr]) -> Vec<u8> {
    let n = addrs.len().min(MAX_ADDRS);
    let mut out = vec![n as u8];
    for a in &addrs[..n] {
        out.extend_from_slice(&encode_addr(a));
    }
    out
}

fn decode_addrs(b: &[u8]) -> Option<Vec<SocketAddr>> {
    let (&n, rest) = b.split_first()?;
    let n = n as usize;
    if n == 0 || n > MAX_ADDRS || rest.len() != n * ADDR_BYTES {
        return None;
    }
    rest.chunks(ADDR_BYTES).map(|ch| ch.try_into().ok().and_then(|a: [u8; ADDR_BYTES]| decode_addr(&a))).collect()
}

/// Federacao: `[1 n][n nome]` depois da oferta da perna (como no tipo 11).
fn relay_tail(tail: &[u8]) -> Option<Option<String>> {
    match tail {
        [] => Some(None),
        [n, name @ ..] if *n as usize == name.len() && (1..=32).contains(&name.len()) => {
            if name.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-') {
                Some(Some(String::from_utf8(name.to_vec()).ok()?))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Os outros membros do grupo (ids das conversas) e o meu id, se eu fizer parte dele.
fn members(c: &st::rusqlite::Connection, gid: &str) -> Result<Option<(Vec<String>, String)>, QtError> {
    crate::groups::call_members(c, gid)
}

/// Os membros do grupo da chamada a quem os avisos vao (todos menos eu).
fn others(c: &st::rusqlite::Connection, g: &GroupCall) -> Result<Vec<String>, QtError> {
    Ok(members(c, &g.group)?.map(|(o, _)| o).unwrap_or_default())
}

impl Leg {
    fn new(id: [u8; 16], role: Role, offer_secret: [u8; 32], pad: u16, relay: Option<String>) -> Leg {
        Leg {
            id,
            role,
            room: RoomKeys::derive(&offer_secret, &id),
            offer_secret: Zeroizing::new(offer_secret),
            pad,
            relay,
            media: None,
            answered: false,
            addrs_sent: false,
            peer_addrs: None,
            peer_camera: false,
            peer_name: None,
            #[cfg(feature = "calls")]
            audio: None,
            #[cfg(feature = "calls")]
            heard: 0,
            #[cfg(feature = "calls")]
            heard_at: now_ms(),
        }
    }

    fn derive_media(&self, answer_secret: &[u8; 32]) -> Result<MediaSession, QtError> {
        let keys = CallKeys::derive(&self.offer_secret, answer_secret, &self.id, self.role);
        MediaSession::new(keys, self.pad as usize).map_err(|_| QtError::Crypto)
    }

    /// Chaves da perna: para o motor (se existir) ou ficam a espera dele.
    fn set_media(&mut self, media: MediaSession) {
        self.answered = true;
        #[cfg(feature = "calls")]
        if let Some(a) = &self.audio {
            a.attach(media);
            a.p2p_enable();
            return;
        }
        self.media = Some(media);
    }

    /// O video desta perna: sai com a minha camara ligada; entra com a dele ligada (entrar na
    /// chamada de grupo e aceitar ver).
    #[cfg(feature = "calls")]
    fn apply_video(&self, camera: bool) {
        if let Some(a) = &self.audio {
            a.video_authorize(camera, self.peer_camera);
        }
    }

    #[cfg(feature = "calls")]
    fn stop(&self) {
        if let Some(a) = &self.audio {
            a.stop();
        }
    }
}

impl GroupCall {
    /// Cria a perna com `peer` e devolve a oferta (eu sou o id menor).
    fn offer_leg(&mut self, own_relay: &Option<String>, peer: &str) -> Result<Vec<u8>, QtError> {
        let leg_id = random::<16>()?;
        let secret = random::<32>()?;
        let mut extra = Zeroizing::new(leg_id.to_vec());
        extra.extend_from_slice(&secret);
        extra.extend_from_slice(&crate::call::DEFAULT_PAD.to_be_bytes());
        if let Some(n) = own_relay {
            extra.push(n.len() as u8);
            extra.extend_from_slice(n.as_bytes());
        }
        if let Some(old) = self.legs.remove(peer) {
            #[cfg(feature = "calls")]
            old.stop();
            #[cfg(not(feature = "calls"))]
            drop(old);
        }
        self.legs.insert(peer.to_string(), Leg::new(leg_id, Role::Caller, secret, crate::call::DEFAULT_PAD, own_relay.clone()));
        Ok(encode(OP_LEG_OFFER, &self.id, &extra))
    }
}

/// Comigo, a chamada passava do maximo?
fn full(g: &GroupCall) -> bool {
    g.present.len() + 1 > MAX_GROUP_CALL
}

/// Tipo 14 recebido de `cid` (ja decifrado, dentro da transacao da rececao).
pub(crate) fn on_group_call(client: &QtClient, c: &st::rusqlite::Connection, cid: &str, body: &[u8], sent_at: i64) -> Result<ReceiveResult, QtError> {
    let no_change = ReceiveResult::NoChange { contact_id: cid.to_string() };
    if body.len() < 17 || is_blocked_in(c, cid)? {
        return Ok(no_change);
    }
    let op = body[0];
    let id: [u8; 16] = body[1..17].try_into().map_err(|_| QtError::InvalidEnvelope)?;
    let rest = &body[17..];
    // Uma chamada a dois a decorrer: ocupado (a ordem dos locks e sempre chamadas -> grupo).
    let (one_to_one, own_relay) = {
        let calls = crate::client::relock(&client.calls);
        (calls.in_call(), calls.own_relay.clone())
    };
    let mut gc = crate::client::relock(&client.group_calls);
    // Um toque antigo (recebido em segundo plano, sem `group_call_events`) acaba antes de
    // decidir: senao um convite novo dava "ocupado".
    expire(&mut gc);
    if gc.ended.contains(&id) {
        return Ok(no_change);
    }
    let now = now_ms();
    let is_cur = gc.cur.as_ref().is_some_and(|g| g.id == id);

    match op {
        OP_INVITE | OP_JOINED => {
            if rest.len() < 16 {
                return Ok(no_change);
            }
            let gid = hex(&rest[..16]);
            // So de membros do grupo, num grupo de que faco parte.
            let Some((others, _)) = members(c, &gid)? else { return Ok(no_change) };
            if !others.iter().any(|o| o == cid) {
                return Ok(no_change);
            }
            if is_cur {
                let Some(g) = gc.cur.as_mut() else { return Ok(no_change) };
                if g.group != gid {
                    return Ok(no_change);
                }
                let new = g.present.insert(cid.to_string());
                if op == OP_JOINED && g.state == State::Joined && !g.legs.contains_key(cid) && g.legs.len() + 1 < MAX_GROUP_CALL {
                    let (_, me) = members(c, &gid)?.unwrap_or_default();
                    if me.as_str() < cid {
                        let plain = g.offer_leg(&own_relay, cid)?;
                        queue(client, c, cid, &plain, None)?;
                        #[cfg(feature = "calls")]
                        if let Some(a) = gc.audio.clone() {
                            start_leg_engine(&mut gc, &a, cid);
                        }
                    } else if new {
                        // Quem oferece e ele (id menor), mas pode nao saber que eu ja estou: o meu
                        // "entrei" pode nao lhe ter chegado (entrei antes de termos sessao). Repito-o
                        // so a ele, uma vez (0.9.67).
                        let mut extra = rest[..16].to_vec();
                        extra.truncate(16);
                        queue(client, c, cid, &encode(OP_JOINED, &id, &extra), None)?;
                    }
                }
                if new {
                    gc.changed();
                }
                return Ok(no_change);
            }
            // Uma chamada que recusei: so muda quem esta (nao volta a tocar).
            if let Some(k) = gc.known.get_mut(&id) {
                k.1.insert(cid.to_string());
                k.3 = now;
                gc.changed_known(&gid);
                return Ok(no_change);
            }
            // Um convite (ou "entrei") de uma chamada que nao e a minha.
            if !crate::call::offer_fresh(sent_at, now) {
                return Ok(no_change);
            }
            if gc.cur.is_some() || one_to_one {
                if op == OP_INVITE {
                    gc.remember_ended(id);
                    gc.push(GroupCallEvent::Ended { call_id: hex(&id), group_id: gid, reason: GroupCallEndReason::Busy });
                }
                return Ok(no_change);
            }
            let video = op == OP_INVITE && rest.get(16).is_some_and(|f| f & 1 == 1);
            let mut present = BTreeSet::new();
            present.insert(cid.to_string());
            gc.cur = Some(GroupCall {
                id,
                group: gid.clone(),
                state: State::Ringing,
                video,
                since: now,
                present,
                legs: BTreeMap::new(),
                camera: false,
                my_name: None,
                waiting: BTreeMap::new(),
            });
            gc.push(GroupCallEvent::Incoming { call_id: hex(&id), group_id: gid, from: cid.to_string(), video });
            Ok(no_change)
        }
        OP_LEFT => {
            if !is_cur {
                if let Some(k) = gc.known.get_mut(&id) {
                    k.1.remove(cid);
                    k.3 = now;
                    let gid = k.0.clone();
                    if k.1.is_empty() {
                        gc.known.remove(&id);
                        gc.remember_ended(id);
                    }
                    gc.changed_known(&gid);
                }
                return Ok(no_change);
            }
            let Some(g) = gc.cur.as_mut() else { return Ok(no_change) };
            g.present.remove(cid);
            if let Some(leg) = g.legs.remove(cid) {
                #[cfg(feature = "calls")]
                leg.stop();
                #[cfg(not(feature = "calls"))]
                drop(leg);
            }
            let ringing_alone = g.state == State::Ringing && g.present.is_empty();
            // A perna sai tambem do audio do grupo (mistura, video e quem fala).
            #[cfg(feature = "calls")]
            if let Some(a) = &gc.audio {
                a.remove_leg(cid);
            }
            // A tocar e ja nao ha ninguem na chamada: para de tocar (chamada perdida).
            if ringing_alone {
                gc.end(GroupCallEndReason::Missed);
            } else {
                gc.changed();
            }
            Ok(no_change)
        }
        OP_LEG_OFFER => {
            if !is_cur || rest.len() < 16 + 32 + 2 {
                return Ok(no_change);
            }
            let Some(relay) = relay_tail(&rest[50..]) else { return Ok(no_change) };
            let Some(g) = gc.cur.as_mut() else { return Ok(no_change) };
            let Some((others, me)) = members(c, &g.group)? else { return Ok(no_change) };
            // So de membros do grupo, como o convite e o "entrei": quem sabe o id da chamada mas
            // nao e do grupo (ex.: tirado do grupo a meio) nao entra na malha nem fica com o meu IP.
            if !others.iter().any(|o| o == cid) {
                return Ok(no_change);
            }
            // Oferece sempre o id menor; so com a chamada aceite e com lugar.
            if g.state != State::Joined || cid >= me.as_str() || (!g.legs.contains_key(cid) && g.legs.len() + 1 >= MAX_GROUP_CALL) {
                return Ok(no_change);
            }
            let pad = u16::from_be_bytes([rest[48], rest[49]]);
            if !(MIN_PAD..=MAX_PAD).contains(&(pad as usize)) {
                return Ok(no_change);
            }
            let leg_id: [u8; 16] = rest[..16].try_into().map_err(|_| QtError::InvalidEnvelope)?;
            let secret: [u8; 32] = rest[16..48].try_into().map_err(|_| QtError::InvalidEnvelope)?;
            if let Some(old) = g.legs.remove(cid) {
                #[cfg(feature = "calls")]
                old.stop();
                #[cfg(not(feature = "calls"))]
                drop(old);
            }
            let mut leg = Leg::new(leg_id, Role::Callee, secret, pad, relay);
            let answer = Zeroizing::new(random::<32>()?);
            let media = leg.derive_media(&answer)?;
            leg.set_media(media);
            let mut extra = Zeroizing::new(leg_id.to_vec());
            extra.extend_from_slice(&answer[..]);
            queue(client, c, cid, &encode(OP_LEG_ANSWER, &g.id, &extra), None)?;
            if g.camera {
                queue(client, c, cid, &encode(OP_CAMERA, &g.id, &[1]), None)?;
            }
            if let Some(n) = &g.my_name {
                queue(client, c, cid, &encode(OP_NAME, &g.id, &name_body(n)), None)?;
            }
            g.present.insert(cid.to_string());
            g.legs.insert(cid.to_string(), leg);
            #[cfg(feature = "calls")]
            if let Some(a) = gc.audio.clone() {
                start_leg_engine(&mut gc, &a, cid);
            }
            gc.changed();
            Ok(no_change)
        }
        OP_LEG_ANSWER => {
            if !is_cur || rest.len() != 16 + 32 {
                return Ok(no_change);
            }
            let Some(g) = gc.cur.as_mut() else { return Ok(no_change) };
            let camera = g.camera;
            let gid_bytes = g.id;
            let my_name = g.my_name.clone();
            let Some(leg) = g.legs.get_mut(cid) else { return Ok(no_change) };
            if leg.id[..] != rest[..16] || leg.role != Role::Caller || leg.answered {
                return Ok(no_change);
            }
            let answer: [u8; 32] = rest[16..48].try_into().map_err(|_| QtError::InvalidEnvelope)?;
            let media = leg.derive_media(&answer)?;
            leg.set_media(media);
            if camera {
                queue(client, c, cid, &encode(OP_CAMERA, &gid_bytes, &[1]), None)?;
            }
            if let Some(n) = &my_name {
                queue(client, c, cid, &encode(OP_NAME, &gid_bytes, &name_body(n)), None)?;
            }
            gc.changed();
            Ok(no_change)
        }
        OP_ADDRS => {
            if !is_cur || rest.len() < 17 {
                return Ok(no_change);
            }
            let Some(addrs) = decode_addrs(&rest[16..]) else { return Ok(no_change) };
            let Some(g) = gc.cur.as_mut() else { return Ok(no_change) };
            let Some(leg) = g.legs.get_mut(cid) else { return Ok(no_change) };
            if leg.id[..] != rest[..16] || !leg.answered {
                return Ok(no_change);
            }
            #[cfg(feature = "calls")]
            if let Some(a) = &leg.audio {
                a.p2p_set_peer(addrs.clone());
            }
            leg.peer_addrs = Some(addrs);
            Ok(no_change)
        }
        OP_NAME => {
            if !is_cur {
                return Ok(no_change);
            }
            let Some(name) = parse_name(rest) else { return Ok(no_change) };
            let Some(g) = gc.cur.as_mut() else { return Ok(no_change) };
            let Some(leg) = g.legs.get_mut(cid) else { return Ok(no_change) };
            if leg.peer_name.as_deref() != Some(name.as_str()) {
                leg.peer_name = Some(name);
                gc.changed();
            }
            Ok(no_change)
        }
        OP_CAMERA => {
            if !is_cur || rest.is_empty() {
                return Ok(no_change);
            }
            let Some(g) = gc.cur.as_mut() else { return Ok(no_change) };
            let camera = g.camera;
            let Some(leg) = g.legs.get_mut(cid) else { return Ok(no_change) };
            let on = rest[0] & 1 == 1;
            if leg.peer_camera != on {
                leg.peer_camera = on;
                #[cfg(feature = "calls")]
                leg.apply_video(camera);
                #[cfg(not(feature = "calls"))]
                let _ = camera;
                gc.changed();
            }
            Ok(no_change)
        }
        _ => Ok(no_change),
    }
}

/// Arranca o motor da perna com `peer` (o audio do grupo ja existe).
#[cfg(feature = "calls")]
fn start_leg_engine(gc: &mut GroupCalls, audio: &Arc<GroupAudio>, peer: &str) {
    let Some(g) = gc.cur.as_mut() else { return };
    let camera = g.camera;
    let Some(leg) = g.legs.get_mut(peer) else { return };
    if leg.audio.is_some() {
        return;
    }
    let Some((url, key)) = audio.relay_for(leg.relay.as_deref()) else { return };
    let Ok(cfg) = crate::call_audio::engine_config(&url) else { return };
    let Ok(join) = qt_call::engine::join_packet(&key, leg.role, &leg.room) else { return };
    let Ok(engine) = qt_call::engine::CallEngine::start_pending(cfg, leg.room.clone(), join) else { return };
    let a = crate::call_audio::CallAudio::new(engine);
    if let Some(m) = leg.media.take() {
        a.attach(m);
    }
    if leg.answered {
        a.p2p_enable();
        if let Some(p) = &leg.peer_addrs {
            a.p2p_set_peer(p.clone());
        }
    }
    a.video_authorize(camera, leg.peer_camera);
    audio.add_leg(peer, a.clone());
    leg.audio = Some(a);
    // O prazo de `LEG_SILENT_MS` conta a partir de agora (antes nao havia nada para ouvir).
    leg.heard_at = now_ms();
}

/// Quem saiu sem avisar (`LEG_SILENT_MS`) sai da chamada, como com "sai". So com o motor de
/// audio: e ele que diz se ainda chega alguma coisa do outro lado.
#[cfg(feature = "calls")]
fn drop_silent(gc: &mut GroupCalls) {
    let now = now_ms();
    #[cfg(test)]
    let timeout = gc.leg_timeout.unwrap_or(LEG_SILENT_MS);
    #[cfg(not(test))]
    let timeout = LEG_SILENT_MS;
    let Some(g) = gc.cur.as_mut() else { return };
    if g.state != State::Joined {
        return;
    }
    let mut gone = Vec::new();
    for (peer, leg) in g.legs.iter_mut() {
        // Sem motor (a app ainda nao arrancou o audio): ainda nao ha nada para ouvir.
        let Some(a) = &leg.audio else { continue };
        if let Some(n) = a.heard() {
            if n != leg.heard {
                leg.heard = n;
                leg.heard_at = now;
            }
        }
        if now - leg.heard_at > timeout {
            gone.push(peer.clone());
        }
    }
    // Disse "entrei" e era ele a oferecer a perna, mas ela nunca chegou.
    let waiting: Vec<String> = g.present.iter().filter(|p| !g.legs.contains_key(*p)).cloned().collect();
    g.waiting.retain(|p, _| waiting.contains(p));
    for p in waiting {
        let since = *g.waiting.entry(p.clone()).or_insert(now);
        if now - since > timeout {
            gone.push(p);
        }
    }
    if gone.is_empty() {
        return;
    }
    for p in &gone {
        g.present.remove(p);
        g.waiting.remove(p);
        if let Some(leg) = g.legs.remove(p) {
            leg.stop();
        }
        if let Some(a) = &gc.audio {
            a.remove_leg(p);
        }
    }
    gc.changed();
}

/// Expira o toque (sem ninguem a atender em 45 s, como nas chamadas a dois).
fn expire(gc: &mut GroupCalls) {
    gc.prune_known();
    let ring_over = gc.cur.as_ref().is_some_and(|g| g.state == State::Ringing && now_ms() - g.since > crate::call::RING_TIMEOUT_MS);
    if ring_over {
        gc.end(GroupCallEndReason::Missed);
    }
}

#[uniffi::export]
impl QtClient {
    /// O aviso "Nas chamadas de grupo, o teu endereco de rede fica visivel aos outros
    /// participantes" ja foi aceite? Sem isso nao se liga nem se entra.
    pub fn group_call_notice_accepted(&self) -> Result<bool, QtError> {
        let c = self.lock()?;
        notice_accepted(&c)
    }

    /// "Aceitar" no aviso (fica guardado).
    pub fn group_call_accept_notice(&self) -> Result<(), QtError> {
        let c = self.lock()?;
        st::kv_set(&c, KV_NOTICE, &[1])?;
        Ok(())
    }

    /// LIGAR a um grupo (de que faco parte). Os convites ficam na outbox (tocam a campainha
    /// de cada membro). Eu fico ja na chamada, a espera de quem entrar.
    pub fn group_call_start(&self, group_id: String, video: bool) -> Result<GroupCallStart, QtError> {
        let c = self.lock()?;
        if !notice_accepted(&c)? {
            return Err(bad("group_call_notice"));
        }
        if crate::client::relock(&self.calls).in_call() {
            return Err(bad("call_busy"));
        }
        let mut gc = crate::client::relock(&self.group_calls);
        expire(&mut gc);
        if gc.cur.is_some() {
            return Err(bad("call_busy"));
        }
        let gid = unhex(&group_id).filter(|b| b.len() == 16).ok_or_else(|| bad("group_id"))?;
        let Some((others, _)) = members(&c, &group_id)? else { return Err(bad("group_id")) };
        let id = random::<16>()?;
        let mut extra = gid.clone();
        extra.push(video as u8);
        let tx = c.unchecked_transaction()?;
        let items = broadcast(self, &tx, &others, &encode(OP_INVITE, &id, &extra), Some(CallWake::Call))?;
        tx.commit()?;
        if items.is_empty() {
            return Err(bad("group_id"));
        }
        gc.cur = Some(GroupCall {
            id,
            group: group_id,
            state: State::Joined,
            video,
            since: now_ms(),
            present: BTreeSet::new(),
            legs: BTreeMap::new(),
            camera: video,
            my_name: None,
            waiting: BTreeMap::new(),
        });
        Ok(GroupCallStart { call_id: hex(&id), items })
    }

    /// A chamada de grupo a decorrer neste grupo em que nao estou (recusei ou nao atendi), se
    /// houver: a conversa do grupo mostra "Entrar na chamada" (0.9.65).
    pub fn group_call_ongoing(&self, group_id: String) -> Result<Option<String>, QtError> {
        let mut gc = crate::client::relock(&self.group_calls);
        gc.prune_known();
        Ok(gc.known.iter().find(|(_, k)| k.0 == group_id && !k.1.is_empty()).map(|(id, _)| hex(id)))
    }

    /// ENTRAR DEPOIS numa chamada que recusei (ou nao atendi): como atender o convite.
    pub fn group_call_rejoin(&self, call_id: String) -> Result<(), QtError> {
        let id = parse_id(&call_id)?;
        {
            if crate::client::relock(&self.calls).in_call() {
                return Err(bad("call_busy"));
            }
            let mut gc = crate::client::relock(&self.group_calls);
            if gc.cur.is_some() {
                return Err(bad("call_busy"));
            }
            let Some((group, present, video, _)) = gc.known.remove(&id) else { return Err(bad("call_id")) };
            gc.cur = Some(GroupCall {
                id,
                group,
                state: State::Ringing,
                video,
                since: now_ms(),
                present,
                legs: BTreeMap::new(),
                camera: false,
                my_name: None,
                waiting: BTreeMap::new(),
            });
        }
        self.group_call_join(call_id)
    }

    /// ENTRAR na chamada de grupo que esta a tocar. Os avisos e as ofertas ficam na outbox.
    pub fn group_call_join(&self, call_id: String) -> Result<(), QtError> {
        let id = parse_id(&call_id)?;
        let c = self.lock()?;
        if !notice_accepted(&c)? {
            return Err(bad("group_call_notice"));
        }
        // Uma chamada de cada vez: com uma chamada a dois (a tocar ou atendida) nao se entra.
        let (one_to_one, own_relay) = {
            let calls = crate::client::relock(&self.calls);
            (calls.in_call(), calls.own_relay.clone())
        };
        if one_to_one {
            return Err(bad("call_busy"));
        }
        let mut gc = crate::client::relock(&self.group_calls);
        expire(&mut gc);
        let g = match gc.cur.as_mut() {
            Some(g) if g.id == id && g.state == State::Ringing => g,
            _ => return Err(bad("call_id")),
        };
        if full(g) {
            return Err(bad("group_call_full"));
        }
        let Some((others, me)) = members(&c, &g.group)? else { return Err(bad("group_id")) };
        let gid = unhex(&g.group).unwrap_or_default();
        let tx = c.unchecked_transaction()?;
        broadcast(self, &tx, &others, &encode(OP_JOINED, &id, &gid), None)?;
        g.state = State::Joined;
        g.since = now_ms();
        // A videochamada: a minha camara comeca ligada, como a de quem ligou.
        g.camera = g.video;
        let present: Vec<String> = g.present.iter().cloned().collect();
        let mut offered = Vec::new();
        for peer in present {
            if me < peer && !is_blocked_in(&tx, &peer)? && st::load_session(&tx, &peer)?.is_some() {
                let plain = g.offer_leg(&own_relay, &peer)?;
                queue(self, &tx, &peer, &plain, None)?;
                offered.push(peer);
            }
        }
        tx.commit()?;
        #[cfg(feature = "calls")]
        if let Some(a) = gc.audio.clone() {
            for p in &offered {
                start_leg_engine(&mut gc, &a, p);
            }
        }
        #[cfg(not(feature = "calls"))]
        drop(offered);
        gc.changed();
        Ok(())
    }

    /// SAIR (ou recusar o convite). Ao sair, o aviso vai para todos os membros.
    pub fn group_call_leave(&self, call_id: String) -> Result<(), QtError> {
        let id = parse_id(&call_id)?;
        let c = self.lock()?;
        let mut gc = crate::client::relock(&self.group_calls);
        let Some(g) = gc.cur.as_ref().filter(|g| g.id == id) else { return Ok(()) };
        if g.state == State::Joined {
            let to = others(&c, g)?;
            let gid = unhex(&g.group).unwrap_or_default();
            // O ultimo a sair para a campainha de quem ainda esta a tocar.
            let wake = g.present.is_empty().then_some(CallWake::CallEnd);
            let tx = c.unchecked_transaction()?;
            broadcast(self, &tx, &to, &encode(OP_LEFT, &id, &gid), wake)?;
            tx.commit()?;
        }
        if let Some(g) = gc.cur.as_ref() {
            #[cfg(feature = "calls")]
            for leg in g.legs.values() {
                leg.stop();
            }
            #[cfg(not(feature = "calls"))]
            let _ = g;
        }
        gc.end(GroupCallEndReason::Left);
        Ok(())
    }

    /// O MEU NOME nesta chamada (0.9.67): o que os outros veem quando nao me tem nos contactos.
    /// Chamar logo a seguir a ligar ou entrar; vai cifrado a cada participante (e aos que
    /// entrarem depois). Vazio: nenhum.
    pub fn group_call_set_name(&self, call_id: String, name: String) -> Result<(), QtError> {
        let id = parse_id(&call_id)?;
        let name = clean_name(&name);
        let c = self.lock()?;
        let mut gc = crate::client::relock(&self.group_calls);
        let g = match gc.cur.as_mut() {
            Some(g) if g.id == id && g.state == State::Joined => g,
            _ => return Err(bad("call_id")),
        };
        if g.my_name == name {
            return Ok(());
        }
        g.my_name = name.clone();
        if let Some(n) = name {
            let tx = c.unchecked_transaction()?;
            for (peer, leg) in &g.legs {
                if leg.answered {
                    queue(self, &tx, peer, &encode(OP_NAME, &id, &name_body(&n)), None)?;
                }
            }
            tx.commit()?;
        }
        Ok(())
    }

    /// Ligar ou desligar a minha camara (vai para cada perna).
    pub fn group_call_set_camera(&self, call_id: String, on: bool) -> Result<(), QtError> {
        let id = parse_id(&call_id)?;
        let c = self.lock()?;
        let mut gc = crate::client::relock(&self.group_calls);
        let g = match gc.cur.as_mut() {
            Some(g) if g.id == id && g.state == State::Joined => g,
            _ => return Err(bad("call_id")),
        };
        if g.camera == on {
            return Ok(());
        }
        g.camera = on;
        let tx = c.unchecked_transaction()?;
        for (peer, leg) in &g.legs {
            #[cfg(feature = "calls")]
            leg.apply_video(on);
            if leg.answered {
                queue(self, &tx, peer, &encode(OP_CAMERA, &id, &[on as u8]), None)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Eventos das chamadas de grupo desde a ultima vez. Chamar depois de cada lote de
    /// `receive_sealed` e de segundo a segundo durante a chamada: expira o toque e envia os
    /// meus enderecos de cada perna quando o motor os tem.
    pub fn group_call_events(&self) -> Result<Vec<GroupCallEvent>, QtError> {
        let c = self.lock()?;
        let mut gc = crate::client::relock(&self.group_calls);
        expire(&mut gc);
        #[cfg(feature = "calls")]
        drop_silent(&mut gc);
        #[cfg(feature = "calls")]
        if let Some(g) = gc.cur.as_mut() {
            let tx = c.unchecked_transaction()?;
            for (peer, leg) in g.legs.iter_mut() {
                if !leg.answered || leg.addrs_sent {
                    continue;
                }
                let Some(local) = leg.audio.as_ref().and_then(|a| a.p2p_local_candidates()) else { continue };
                leg.addrs_sent = true;
                if !local.is_empty() {
                    let mut extra = leg.id.to_vec();
                    extra.extend_from_slice(&encode_addrs(&local));
                    queue(self, &tx, peer, &encode(OP_ADDRS, &g.id, &extra), None)?;
                }
            }
            tx.commit()?;
        }
        #[cfg(not(feature = "calls"))]
        let _ = &c;
        Ok(gc.events.drain(..).collect())
    }

    /// A chamada de grupo a decorrer (a tocar ou dentro), se houver.
    pub fn group_call_current(&self) -> Result<Option<String>, QtError> {
        Ok(crate::client::relock(&self.group_calls).cur.as_ref().map(|g| hex(&g.id)))
    }

    /// O grupo da chamada.
    pub fn group_call_group(&self, call_id: String) -> Result<Option<String>, QtError> {
        let id = parse_id(&call_id)?;
        Ok(crate::client::relock(&self.group_calls).cur.as_ref().filter(|g| g.id == id).map(|g| g.group.clone()))
    }

    /// Videochamada de grupo (a camara de quem liga comeca ligada)?
    pub fn group_call_is_video(&self, call_id: String) -> Result<bool, QtError> {
        let id = parse_id(&call_id)?;
        Ok(crate::client::relock(&self.group_calls).cur.as_ref().is_some_and(|g| g.id == id && g.video))
    }

    /// Quem esta na chamada (sem mim), pela ordem dos ids.
    pub fn group_call_participants(&self, call_id: String) -> Result<Vec<GroupCallParticipant>, QtError> {
        let id = parse_id(&call_id)?;
        let gc = crate::client::relock(&self.group_calls);
        let Some(g) = gc.cur.as_ref().filter(|g| g.id == id) else { return Ok(Vec::new()) };
        Ok(g
            .present
            .iter()
            .map(|p| {
                let leg = g.legs.get(p);
                GroupCallParticipant {
                    contact_id: p.clone(),
                    connected: leg.is_some_and(|l| l.answered),
                    camera: leg.is_some_and(|l| l.peer_camera),
                    name: leg.and_then(|l| l.peer_name.clone()),
                }
            })
            .collect())
    }
}

// ============================================================================ audio do grupo

/// Nivel (RMS) a partir do qual alguem esta a falar.
#[cfg(feature = "calls")]
const SPEAKING_RMS: f64 = 600.0;

/// O AUDIO E O VIDEO da chamada de grupo: as pernas, a mistura do que se ouve, quem esta a
/// falar e o video para todos. A app usa-o como o `CallAudio` das chamadas a dois.
#[cfg(feature = "calls")]
#[derive(uniffi::Object)]
pub struct GroupAudio {
    legs: Mutex<Vec<(String, Arc<crate::call_audio::CallAudio>)>>,
    /// O relay principal e os outros da federacao (nome -> URL e pin).
    relay: (String, Vec<u8>),
    others: Vec<CallRelayInfo>,
    st: Mutex<GroupAudioState>,
}

#[cfg(feature = "calls")]
#[derive(Default)]
struct GroupAudioState {
    /// Nivel recente de cada participante e o meu (media exponencial do RMS).
    levels: BTreeMap<String, f64>,
    my_level: f64,
    /// Estou a falar (com histerese: 1,5 s para subir, 4 s para descer).
    talking: bool,
    talk_since: Option<std::time::Instant>,
    quiet_since: Option<std::time::Instant>,
    speaker: Option<String>,
    speaker_at: Option<std::time::Instant>,
    encode_limit: u32,
    decode_limit: u32,
    net: crate::call_audio::NetworkKind,
    hold: bool,
    stopped: bool,
    key_sum: u32,
}

#[cfg(feature = "calls")]
fn rms(pcm: &[u8]) -> f64 {
    let n = pcm.len() / 2;
    if n == 0 {
        return 0.0;
    }
    let sum: f64 = pcm.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f64).map(|x| x * x).sum();
    (sum / n as f64).sqrt()
}

#[cfg(feature = "calls")]
impl GroupAudio {
    fn relay_for(&self, name: Option<&str>) -> Option<(String, Vec<u8>)> {
        match name {
            Some(n) => match self.others.iter().find(|r| r.name == n) {
                Some(r) => Some((r.url.clone(), r.public_key.clone())),
                None => Some(self.relay.clone()),
            },
            None => Some(self.relay.clone()),
        }
    }

    fn add_leg(&self, peer: &str, a: Arc<crate::call_audio::CallAudio>) {
        let (enc, dec, net, hold) = {
            let s = crate::client::relock(&self.st);
            (s.encode_limit, s.decode_limit, s.net, s.hold)
        };
        if enc > 0 {
            a.video_set_encode_limit(enc.min(self.policy_height()));
        }
        if dec > 0 {
            a.video_set_decode_limit(dec);
        }
        a.set_network(net);
        a.video_hold(hold);
        let mut legs = crate::client::relock(&self.legs);
        legs.retain(|(p, _)| p != peer);
        legs.push((peer.to_string(), a));
    }

    /// `peer` saiu: a perna para e deixa de contar para a mistura, o video e quem fala.
    fn remove_leg(&self, peer: &str) {
        let removed = {
            let mut legs = crate::client::relock(&self.legs);
            legs.iter().position(|(p, _)| p == peer).map(|i| legs.remove(i).1)
        };
        if let Some(a) = removed {
            a.stop();
        }
        let mut s = crate::client::relock(&self.st);
        s.levels.remove(peer);
        if s.speaker.as_deref() == Some(peer) {
            s.speaker = None;
            s.speaker_at = None;
        }
    }

    fn leg(&self, peer: &str) -> Option<Arc<crate::call_audio::CallAudio>> {
        crate::client::relock(&self.legs).iter().find(|(p, _)| p == peer).map(|(_, a)| a.clone())
    }

    fn all(&self) -> Vec<(String, Arc<crate::call_audio::CallAudio>)> {
        crate::client::relock(&self.legs).clone()
    }

    /// Teto do meu video no grupo: 360p; a falar, 720p no Wi-Fi e 480p com dados.
    fn policy_height(&self) -> u32 {
        let s = crate::client::relock(&self.st);
        match (s.talking, s.net) {
            (false, _) => 360,
            (true, crate::call_audio::NetworkKind::Wifi) => 720,
            (true, _) => 480,
        }
    }

    pub(crate) fn stop(&self) {
        crate::client::relock(&self.st).stopped = true;
        for (_, a) in self.all() {
            a.stop();
        }
    }
}

#[cfg(feature = "calls")]
#[uniffi::export]
impl GroupAudio {
    /// 20 ms do microfone (1920 bytes): vai para todas as pernas.
    pub fn push_capture(&self, pcm: Vec<u8>) -> Result<(), QtError> {
        if pcm.len() != crate::call_audio::CALL_FRAME_BYTES as usize {
            return Err(bad("pcm"));
        }
        let level = rms(&pcm);
        {
            let mut s = crate::client::relock(&self.st);
            s.my_level = s.my_level * 0.9 + level * 0.1;
            let now = std::time::Instant::now();
            if s.my_level >= SPEAKING_RMS {
                s.quiet_since = None;
                let since = *s.talk_since.get_or_insert(now);
                if now.duration_since(since) >= std::time::Duration::from_millis(1500) {
                    s.talking = true;
                }
            } else {
                s.talk_since = None;
                let since = *s.quiet_since.get_or_insert(now);
                if now.duration_since(since) >= std::time::Duration::from_secs(4) {
                    s.talking = false;
                }
            }
        }
        for (_, a) in self.all() {
            a.push_capture(pcm.clone())?;
        }
        Ok(())
    }

    /// Os proximos 20 ms para o altifalante: a mistura de todas as pernas.
    pub fn pull_playback(&self) -> Vec<u8> {
        let n = crate::call_audio::CALL_FRAME_BYTES as usize / 2;
        let mut acc = vec![0i32; n];
        let mut levels = Vec::new();
        for (peer, a) in self.all() {
            let pcm = a.pull_playback();
            levels.push((peer, rms(&pcm)));
            for (i, b) in pcm.chunks_exact(2).take(n).enumerate() {
                acc[i] += i16::from_le_bytes([b[0], b[1]]) as i32;
            }
        }
        {
            let mut s = crate::client::relock(&self.st);
            let now = std::time::Instant::now();
            for (p, l) in levels {
                let e = s.levels.entry(p).or_insert(0.0);
                *e = *e * 0.9 + l * 0.1;
            }
            // Quem fala mais alto passa a ser o "a falar" (fica pelo menos 1,5 s).
            let loudest = s.levels.iter().filter(|(_, l)| **l >= SPEAKING_RMS).max_by(|a, b| a.1.total_cmp(b.1)).map(|(p, _)| p.clone());
            let hold = s.speaker_at.is_some_and(|t| now.duration_since(t) < std::time::Duration::from_millis(1500));
            if let Some(p) = loudest {
                if s.speaker.as_ref() != Some(&p) && !hold {
                    s.speaker = Some(p);
                    s.speaker_at = Some(now);
                } else if s.speaker.as_ref() == Some(&p) {
                    s.speaker_at = Some(now);
                }
            }
        }
        acc.iter().flat_map(|x| ((*x).clamp(i16::MIN as i32, i16::MAX as i32) as i16).to_le_bytes()).collect()
    }

    /// Quem esta a falar agora (para o quadrado grande). `None`: ninguem ainda.
    pub fn speaker(&self) -> Option<String> {
        crate::client::relock(&self.st).speaker.clone()
    }

    /// Estou eu a falar?
    pub fn me_speaking(&self) -> bool {
        crate::client::relock(&self.st).talking
    }

    /// O que o codificador deve fazer agora (perguntar a cada ~250 ms): o mais pequeno que
    /// todas as pernas que enviam aguentam (uma so codificacao para todos).
    pub fn video_params(&self) -> crate::call_audio::VideoEncoderParams {
        let cap = self.policy_height();
        let (enc, mut key_sum) = {
            let s = crate::client::relock(&self.st);
            (s.encode_limit, 0u32)
        };
        let mut best: Option<crate::call_audio::VideoEncoderParams> = None;
        let mut paused = None;
        for (_, a) in self.all() {
            a.video_set_encode_limit(if enc > 0 { enc.min(cap) } else { cap });
            let p = a.video_params();
            key_sum = key_sum.wrapping_add(p.keyframe_seq);
            if !p.send {
                paused = paused.or(p.paused);
                continue;
            }
            if best.as_ref().is_none_or(|b| p.bitrate_bps < b.bitrate_bps) {
                best = Some(p);
            }
        }
        crate::client::relock(&self.st).key_sum = key_sum;
        match best {
            Some(mut p) => {
                p.keyframe_seq = key_sum;
                p
            }
            None => crate::call_audio::VideoEncoderParams {
                send: false,
                bitrate_bps: 0,
                width: 0,
                height: 0,
                fps: 0,
                paused,
                keyframe_seq: key_sum,
            },
        }
    }

    /// Uma imagem H.264 do codificador: vai para todas as pernas (cada uma so a envia se
    /// puder). `true` se saiu para alguma.
    pub fn video_push(&self, frame: Vec<u8>, keyframe: bool, rotation_degrees: u32, age_ms: u32) -> bool {
        let mut any = false;
        for (_, a) in self.all() {
            any |= a.video_push(frame.clone(), keyframe, rotation_degrees, age_ms);
        }
        any
    }

    /// A proxima imagem de `contact_id`, quando for a hora dela.
    pub fn video_pull(&self, contact_id: String, timeout_ms: u32) -> Option<crate::call_audio::VideoFrameIn> {
        self.leg(&contact_id)?.video_pull(timeout_ms)
    }

    /// O descodificador do video de `contact_id` falhou: pede-lhe uma imagem completa.
    pub fn video_request_keyframe(&self, contact_id: String) {
        if let Some(a) = self.leg(&contact_id) {
            a.video_request_keyframe();
        }
    }

    /// Pausa local da minha camara (a app saiu do ecra).
    pub fn video_hold(&self, on: bool) {
        crate::client::relock(&self.st).hold = on;
        for (_, a) in self.all() {
            a.video_hold(on);
        }
    }

    pub fn video_set_encode_limit(&self, max_height: u32) {
        crate::client::relock(&self.st).encode_limit = max_height;
        let cap = self.policy_height();
        for (_, a) in self.all() {
            a.video_set_encode_limit(max_height.min(cap));
        }
    }

    pub fn video_set_decode_limit(&self, max_height: u32) {
        crate::client::relock(&self.st).decode_limit = max_height;
        for (_, a) in self.all() {
            a.video_set_decode_limit(max_height);
        }
    }

    pub fn set_network(&self, kind: crate::call_audio::NetworkKind) {
        crate::client::relock(&self.st).net = kind;
        for (_, a) in self.all() {
            a.set_network(kind);
        }
    }

    /// Ligacao direta com `contact_id` (so para diagnostico; a app nao mostra nada).
    pub fn direct(&self, contact_id: String) -> bool {
        self.leg(&contact_id).is_some_and(|a| a.stats().direct)
    }

    /// Diagnostico da perna com `contact_id` (sem IPs nem ids: so os numeros). A app junta o
    /// nome de cada um e mostra tudo ao premir o tempo da chamada (como nas chamadas a dois).
    pub fn diagnostic_of(&self, contact_id: String) -> String {
        self.leg(&contact_id).map(|a| a.diagnostic()).unwrap_or_default()
    }

    /// Diagnostico de cada perna (sem IPs nem ids: so os numeros), uma por paragrafo.
    pub fn diagnostic(&self) -> String {
        self.all().iter().enumerate().map(|(i, (_, a))| format!("participante {}\n{}", i + 1, a.diagnostic())).collect::<Vec<_>>().join("\n\n")
    }
}

#[cfg(feature = "calls")]
#[uniffi::export]
impl QtClient {
    /// AUDIO E VIDEO da chamada de grupo: chamar ao ligar ou ao entrar. `relay_url` /
    /// `relay_public_key`: o relay principal; `other_relays`: os outros da federacao (cada
    /// perna usa o de quem a ofereceu). Chamar outra vez devolve o mesmo.
    pub fn group_call_audio_start(
        &self,
        call_id: String,
        relay_url: String,
        relay_public_key: Vec<u8>,
        other_relays: Vec<CallRelayInfo>,
    ) -> Result<Arc<GroupAudio>, QtError> {
        let id = parse_id(&call_id)?;
        if relay_public_key.len() != 32 || other_relays.iter().any(|r| r.public_key.len() != 32) {
            return Err(bad("relay_public_key"));
        }
        crate::call_audio::relay_target(&relay_url)?;
        let mut gc = crate::client::relock(&self.group_calls);
        if !gc.cur.as_ref().is_some_and(|g| g.id == id) {
            return Err(bad("call_id"));
        }
        if let Some(a) = &gc.audio {
            return Ok(a.clone());
        }
        let audio = Arc::new(GroupAudio {
            legs: Mutex::new(Vec::new()),
            relay: (relay_url, relay_public_key),
            others: other_relays,
            st: Mutex::new(GroupAudioState::default()),
        });
        gc.audio = Some(audio.clone());
        let peers: Vec<String> = gc.cur.as_ref().map(|g| g.legs.keys().cloned().collect()).unwrap_or_default();
        for p in peers {
            start_leg_engine(&mut gc, &audio, &p);
        }
        Ok(audio)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enderecos_ida_e_volta() {
        let a: SocketAddr = "192.0.2.1:4000".parse().unwrap();
        let b: SocketAddr = "[2001:db8::1]:5000".parse().unwrap();
        let enc = encode_addrs(&[a, b]);
        assert_eq!(decode_addrs(&enc), Some(vec![a, b]));
        assert_eq!(decode_addrs(&[0]), None);
        assert_eq!(decode_addrs(&[5]), None);
    }

    #[test]
    fn relay_da_perna() {
        assert_eq!(relay_tail(&[]), Some(None));
        assert_eq!(relay_tail(&[7, b'e', b's', b'p', b'a', b'n', b'h', b'a']), Some(Some("espanha".into())));
        assert_eq!(relay_tail(&[2, b'A', b'b']), None);
    }
}

/// Toques e convites sem motores de audio (correm tambem sem a feature `calls`).
#[cfg(test)]
mod ring_tests {
    use super::*;
    use crate::call::CallEvent;
    use crate::client::{peer_bundle_from_relay_bytes, PublicIdentity};

    struct Peer {
        c: Arc<QtClient>,
        id: PublicIdentity,
    }

    fn peer(name: &str) -> Peer {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = format!("/tmp/qt_gring_{name}_{}_{n}.sqlite", std::process::id());
        let _ = std::fs::remove_file(&path);
        let c = QtClient::open(path, vec![0x60 + n as u8; 32]).unwrap();
        let id = c.create_identity().unwrap();
        Peer { c, id }
    }

    fn cid(p: &Peer) -> String {
        hex(&p.id.identity_public)
    }

    fn flush(from: &Peer, all: &[&Peer]) {
        for item in from.c.outbox().unwrap() {
            if let Some(to) = all.iter().find(|p| cid(p) == item.contact_id) {
                to.c.receive_sealed(item.blob.clone()).unwrap();
            }
            from.c.outbox_sent(item.outbox_id.clone()).unwrap();
        }
    }

    fn connect(a: &Peer, b: &Peer) {
        let bundle = peer_bundle_from_relay_bytes(b.c.publishable_bundle().unwrap()).unwrap();
        a.c.start_session(b.id.public_id.clone(), bundle).unwrap();
        a.c.send_text(cid(b), "ola".into()).unwrap();
        flush(a, &[b]);
        b.c.send_text(cid(a), "ola!".into()).unwrap();
        flush(b, &[a]);
    }

    /// Um convite de grupo chega com a app em segundo plano: ninguem chama
    /// `group_call_events`, o toque passa do prazo e o telemovel tem de deixar de estar
    /// "ocupado" (antes, as chamadas a dois e os convites novos davam "ocupado" ate o ecra abrir).
    #[test]
    fn toque_de_grupo_esquecido_nao_deixa_ocupado() {
        let peers: Vec<Peer> = (0..3).map(|i| peer(&format!("r{i}"))).collect();
        for i in 0..3 {
            for j in i + 1..3 {
                connect(&peers[i], &peers[j]);
            }
        }
        let members = peers[1..].iter().map(|p| p.id.public_id.clone()).collect();
        let gid = peers[0].c.group_create("Amigos".into(), members).unwrap().group_id;
        let all: Vec<&Peer> = peers.iter().collect();
        for p in &all {
            flush(p, &all);
        }
        for p in &peers {
            p.c.group_call_accept_notice().unwrap();
        }
        // A liga ao grupo; B recebe o convite em segundo plano (so `receive_sealed`).
        peers[0].c.group_call_start(gid.clone(), false).unwrap();
        flush(&peers[0], &all);
        assert!(peers[1].c.group_call_current().unwrap().is_some(), "o convite chegou a B");
        // O toque passa do prazo sem ninguem perguntar.
        crate::client::relock(&peers[1].c.group_calls).cur.as_mut().unwrap().since -= crate::call::RING_TIMEOUT_MS + 1;
        // C liga a B (chamada a dois): tem de tocar em B, nao "ocupado".
        let call = peers[2].c.call_start(cid(&peers[1])).unwrap();
        flush(&peers[2], &all);
        let ev = peers[1].c.call_events().unwrap();
        assert!(
            ev.iter().any(|e| matches!(e, CallEvent::Incoming { call_id, .. } if *call_id == call.call_id)),
            "a chamada de C toca em B: {ev:?}"
        );
        assert_eq!(peers[1].c.group_call_current().unwrap(), None, "o toque de grupo acabou (perdida)");
        peers[1].c.call_hangup(call.call_id.clone()).unwrap();
        flush(&peers[1], &all);
        let first = peers[0].c.group_call_current().unwrap().unwrap();
        peers[0].c.group_call_leave(first).unwrap();
        flush(&peers[0], &all);
        for p in &peers {
            let _ = p.c.group_call_events().unwrap();
        }

        // O mesmo com um convite de grupo novo durante um toque de grupo esquecido: A liga (X), C
        // recusa, o toque de B passa do prazo sem ninguem perguntar e C liga outra vez (Y).
        let x = peers[0].c.group_call_start(gid.clone(), false).unwrap();
        flush(&peers[0], &all);
        let _ = peers[2].c.group_call_events().unwrap();
        peers[2].c.group_call_leave(x.call_id.clone()).unwrap();
        flush(&peers[2], &all);
        crate::client::relock(&peers[1].c.group_calls).cur.as_mut().unwrap().since -= crate::call::RING_TIMEOUT_MS + 1;
        let y = peers[2].c.group_call_start(gid, false).unwrap();
        flush(&peers[2], &all);
        let ev = peers[1].c.group_call_events().unwrap();
        assert!(
            ev.iter().any(|e| matches!(e, GroupCallEvent::Incoming { call_id, .. } if *call_id == y.call_id)),
            "o convite novo toca em B: {ev:?}"
        );
        assert!(
            !ev.iter().any(|e| matches!(e, GroupCallEvent::Ended { call_id, reason: GroupCallEndReason::Busy, .. } if *call_id == y.call_id)),
            "sem \"ocupado\": {ev:?}"
        );
    }

    /// Alguem que nao e do grupo mas sabe o id da chamada (ex.: tirado do grupo a meio, ou a
    /// quem um membro o passou) e tem sessao com um participante: a oferta de perna dele e
    /// ignorada (antes entrava na malha, ouvia e ficava com o IP desse participante).
    #[test]
    fn oferta_de_perna_de_quem_nao_e_do_grupo_e_ignorada() {
        let peers: Vec<Peer> = (0..3).map(|i| peer(&format!("m{i}"))).collect();
        for i in 0..3 {
            for j in i + 1..3 {
                connect(&peers[i], &peers[j]);
            }
        }
        // O intruso tem de ter um id menor do que alguem do grupo (so o id menor oferece).
        let top = peers.iter().map(cid).max().unwrap();
        let intruder = loop {
            let x = peer("intruso");
            if cid(&x) < top {
                break x;
            }
        };
        let victim = peers.iter().find(|p| cid(p) == top).unwrap();
        connect(&intruder, victim);
        let members = peers[1..].iter().map(|p| p.id.public_id.clone()).collect();
        let gid = peers[0].c.group_create("Amigos".into(), members).unwrap().group_id;
        let all: Vec<&Peer> = peers.iter().collect();
        for p in &all {
            flush(p, &all);
        }
        for p in &peers {
            p.c.group_call_accept_notice().unwrap();
        }
        let start = peers[0].c.group_call_start(gid, false).unwrap();
        flush(&peers[0], &all);
        for p in &peers[1..] {
            p.c.group_call_join(start.call_id.clone()).unwrap();
            flush(p, &all);
        }
        for p in &all {
            flush(p, &all);
        }
        // O intruso oferece uma perna ao participante com o id maior.
        let id: [u8; 16] = unhex(&start.call_id).unwrap().try_into().unwrap();
        let mut extra = vec![9u8; 16 + 32];
        extra.extend_from_slice(&crate::call::DEFAULT_PAD.to_be_bytes());
        {
            let conn = intruder.c.lock().unwrap();
            queue(&intruder.c, &conn, &top, &encode(OP_LEG_OFFER, &id, &extra), None).unwrap();
        }
        flush(&intruder, &[victim]);
        let parts = victim.c.group_call_participants(start.call_id.clone()).unwrap();
        assert!(!parts.iter().any(|p| p.contact_id == cid(&intruder)), "o intruso nao entra: {parts:?}");
        assert_eq!(parts.len(), 2, "so os outros dois membros");
    }

    /// A chamada de grupo toca em B; B liga entretanto a C (chamada a dois) e depois tenta
    /// atender a de grupo: nao pode (antes ficava com as duas chamadas ao mesmo tempo).
    #[test]
    fn nao_se_entra_no_grupo_durante_uma_chamada_a_dois() {
        let peers: Vec<Peer> = (0..3).map(|i| peer(&format!("d{i}"))).collect();
        for i in 0..3 {
            for j in i + 1..3 {
                connect(&peers[i], &peers[j]);
            }
        }
        let members = peers[1..].iter().map(|p| p.id.public_id.clone()).collect();
        let gid = peers[0].c.group_create("Amigos".into(), members).unwrap().group_id;
        let all: Vec<&Peer> = peers.iter().collect();
        for p in &all {
            flush(p, &all);
        }
        for p in &peers {
            p.c.group_call_accept_notice().unwrap();
        }
        let start = peers[0].c.group_call_start(gid, false).unwrap();
        flush(&peers[0], &all);
        assert!(peers[1].c.group_call_current().unwrap().is_some(), "toca em B");
        peers[1].c.call_start(cid(&peers[2])).unwrap();
        assert!(
            matches!(peers[1].c.group_call_join(start.call_id.clone()), Err(QtError::InvalidInput { .. })),
            "B ja esta numa chamada a dois"
        );
        assert_eq!(peers[0].c.group_call_participants(start.call_id).unwrap().len(), 0, "B nao entrou");
    }
}

/// Chamadas de grupo com telemoveis reais (QtClient + motores), sem relay: o audio e o video
/// so podem passar pelos caminhos diretos (127.0.0.1).
#[cfg(all(test, feature = "calls"))]
mod mesh_tests {
    use super::*;
    use crate::client::{peer_bundle_from_relay_bytes, PublicIdentity};
    use std::time::{Duration, Instant};

    struct Peer {
        c: Arc<QtClient>,
        id: PublicIdentity,
    }

    impl Peer {
        fn new(name: &str) -> Peer {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let path = format!("/tmp/qt_gcall_{name}_{}_{n}.sqlite", std::process::id());
            let _ = std::fs::remove_file(&path);
            let c = QtClient::open(path, vec![0x40 + n as u8; 32]).unwrap();
            let id = c.create_identity().unwrap();
            Peer { c, id }
        }
        fn cid(&self) -> String {
            hex(&self.id.identity_public)
        }
    }

    /// Entrega tudo o que `from` tem na outbox (a quem estiver em `all`).
    fn flush(from: &Peer, all: &[&Peer]) {
        for item in from.c.outbox().unwrap() {
            if let Some(to) = all.iter().find(|p| p.cid() == item.contact_id) {
                to.c.receive_sealed(item.blob.clone()).unwrap();
            }
            from.c.outbox_sent(item.outbox_id.clone()).unwrap();
        }
    }

    fn flush_all(all: &[&Peer]) {
        for p in all {
            flush(p, all);
        }
    }

    fn connect(a: &Peer, b: &Peer) {
        let bundle = peer_bundle_from_relay_bytes(b.c.publishable_bundle().unwrap()).unwrap();
        a.c.start_session(b.id.public_id.clone(), bundle).unwrap();
        a.c.send_text(b.cid(), "ola".into()).unwrap();
        flush(a, &[b]);
        b.c.send_text(a.cid(), "ola!".into()).unwrap();
        flush(b, &[a]);
    }

    /// `n` pessoas, todas com sessao umas com as outras, num grupo criado pela primeira.
    fn group(n: usize) -> (Vec<Peer>, String) {
        let peers: Vec<Peer> = (0..n).map(|i| Peer::new(&format!("p{i}"))).collect();
        for i in 0..n {
            for j in i + 1..n {
                connect(&peers[i], &peers[j]);
            }
        }
        let members = peers[1..].iter().map(|p| p.id.public_id.clone()).collect();
        let g = peers[0].c.group_create("Amigos".into(), members).unwrap();
        let all: Vec<&Peer> = peers.iter().collect();
        flush_all(&all);
        for p in &peers {
            p.c.group_call_accept_notice().unwrap();
        }
        (peers, g.group_id)
    }

    fn incoming(p: &Peer) -> Option<String> {
        p.c.group_call_events().unwrap().into_iter().find_map(|e| match e {
            GroupCallEvent::Incoming { call_id, .. } => Some(call_id),
            _ => None,
        })
    }

    fn audio(p: &Peer, id: &str) -> Arc<GroupAudio> {
        let pin = qt_transport::generate_static_keypair().unwrap().public;
        p.c.group_call_audio_start(id.into(), "ws://127.0.0.1:9".into(), pin, Vec::new()).unwrap()
    }

    /// Faz o que a app faz (eventos + outbox) ate `done` ou `secs`.
    fn run(all: &[&Peer], secs: u64, mut done: impl FnMut() -> bool) -> bool {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(secs) {
            for p in all {
                p.c.group_call_events().unwrap();
            }
            flush_all(all);
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    /// Um tom (alto o bastante para contar como fala).
    fn tone(i: usize) -> Vec<u8> {
        (0..960).flat_map(|k| (((((i * 960 + k) as f64) * 0.06).sin() * 8000.0) as i16).to_le_bytes()).collect()
    }

    #[test]
    fn sem_aceitar_o_aviso_nao_se_liga() {
        let (peers, gid) = group(2);
        let p = Peer::new("novo");
        assert!(!p.c.group_call_notice_accepted().unwrap());
        assert!(matches!(p.c.group_call_start(gid.clone(), false), Err(QtError::InvalidInput { .. })));
        assert!(peers[0].c.group_call_notice_accepted().unwrap());
    }

    #[test]
    fn tres_pessoas_ouvem_se_todas_pelo_direto() {
        let (peers, gid) = group(3);
        let all: Vec<&Peer> = peers.iter().collect();
        let start = peers[0].c.group_call_start(gid, false).unwrap();
        assert_eq!(start.items.len(), 2, "um convite para cada membro");
        let a0 = audio(&peers[0], &start.call_id);
        flush_all(&all);
        let (id1, id2) = (incoming(&peers[1]).expect("toca em B"), incoming(&peers[2]).expect("toca em C"));
        assert_eq!((id1.as_str(), id2.as_str()), (start.call_id.as_str(), start.call_id.as_str()));
        peers[1].c.group_call_join(id1.clone()).unwrap();
        let a1 = audio(&peers[1], &id1);
        flush_all(&all);
        peers[2].c.group_call_join(id2.clone()).unwrap();
        let a2 = audio(&peers[2], &id2);
        // Cada um tem uma perna ligada com cada um dos outros dois.
        let auds = [a0.clone(), a1.clone(), a2.clone()];
        let ok = run(&all, 20, || {
            peers.iter().zip(&auds).all(|(p, a)| {
                let parts = p.c.group_call_participants(start.call_id.clone()).unwrap();
                parts.len() == 2 && parts.iter().all(|x| x.connected && a.direct(x.contact_id.clone()))
            })
        });
        assert!(ok, "malha de 3 com ligacao direta em todas as pernas");
        // O nome da chamada: A escolhe "Ana" e B e C passam a ve-lo (cifrado, perna a perna).
        peers[0].c.group_call_set_name(start.call_id.clone(), "  Ana\n ".into()).unwrap();
        let named = run(&all, 5, || {
            peers[1..].iter().all(|p| {
                p.c.group_call_participants(start.call_id.clone()).unwrap().iter().any(|x| x.contact_id == peers[0].cid() && x.name.as_deref() == Some("Ana"))
            })
        });
        assert!(named, "B e C veem o nome que A escolheu");
        assert_eq!(clean_name(&"x".repeat(40)).unwrap().len(), MAX_CALL_NAME);
        assert_eq!(clean_name("   "), None);
        // A fala: B e C ouvem-no (a mistura tem som) e sabem que e ele a falar.
        let t = Instant::now();
        let mut heard = [0usize; 2];
        let mut i = 0;
        while t.elapsed() < Duration::from_secs(4) {
            a0.push_capture(tone(i)).unwrap();
            a1.push_capture(vec![0; 1920]).unwrap();
            a2.push_capture(vec![0; 1920]).unwrap();
            for (k, a) in [&a1, &a2].iter().enumerate() {
                if rms(&a.pull_playback()) > 1000.0 {
                    heard[k] += 1;
                }
            }
            let _ = a0.pull_playback();
            i += 1;
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(heard.iter().all(|h| *h > 50), "B e C ouviram A: {heard:?}");
        assert_eq!(a1.speaker(), Some(peers[0].cid()));
        assert_eq!(a2.speaker(), Some(peers[0].cid()));
        assert!(a0.me_speaking(), "A esta a falar");
        // Quem sai deixa de aparecer aos outros.
        peers[2].c.group_call_leave(id2).unwrap();
        flush_all(&all);
        assert_eq!(peers[0].c.group_call_participants(start.call_id.clone()).unwrap().len(), 1);
        assert!(peers[2].c.group_call_events().unwrap().iter().any(|e| matches!(e, GroupCallEvent::Ended { reason: GroupCallEndReason::Left, .. })));
        for p in &peers[..2] {
            p.c.group_call_leave(start.call_id.clone()).unwrap();
        }
    }

    #[test]
    fn video_vai_para_todos_e_cada_um_ve_os_outros() {
        let (peers, gid) = group(3);
        let all: Vec<&Peer> = peers.iter().collect();
        let start = peers[0].c.group_call_start(gid, true).unwrap();
        let a0 = audio(&peers[0], &start.call_id);
        a0.set_network(crate::call_audio::NetworkKind::Wifi);
        flush_all(&all);
        let mut auds = vec![a0.clone()];
        for p in &peers[1..] {
            let id = incoming(p).unwrap();
            assert!(p.c.group_call_is_video(id.clone()).unwrap());
            p.c.group_call_join(id.clone()).unwrap();
            let a = audio(p, &id);
            a.set_network(crate::call_audio::NetworkKind::Wifi);
            auds.push(a);
            flush_all(&all);
        }
        // Todos com a camara ligada (videochamada): o video de A sai para os dois.
        assert!(run(&all, 20, || a0.video_params().send), "o video de A sai");
        let p = a0.video_params();
        assert!(p.height <= 360, "sem falar, 360p no grupo: {p:?}");
        let t = Instant::now();
        let mut seq = None;
        let mut got = [0usize; 2];
        while t.elapsed() < Duration::from_secs(4) {
            let p = a0.video_params();
            if p.send {
                let key = seq != Some(p.keyframe_seq);
                if a0.video_push(vec![7; if key { 6000 } else { 1500 }], key, 90, 0) && key {
                    seq = Some(p.keyframe_seq);
                }
            }
            for (k, a) in auds[1..].iter().enumerate() {
                if a.video_pull(peers[0].cid(), 5).is_some() {
                    got[k] += 1;
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(got.iter().all(|g| *g > 20), "B e C viram o video de A: {got:?}");
        for p in &peers {
            p.c.group_call_leave(start.call_id.clone()).unwrap();
        }
    }

    #[test]
    fn no_maximo_cinco_e_o_ultimo_a_sair_para_o_toque() {
        let (peers, gid) = group(6);
        let all: Vec<&Peer> = peers.iter().collect();
        let start = peers[0].c.group_call_start(gid, false).unwrap();
        flush_all(&all);
        let ids: Vec<String> = peers[1..].iter().map(|p| incoming(p).unwrap()).collect();
        for (p, id) in peers[1..5].iter().zip(&ids) {
            p.c.group_call_join(id.clone()).unwrap();
            flush_all(&all);
        }
        run(&all, 2, || false);
        // Cinco na chamada: o sexto ja nao entra.
        assert!(matches!(peers[5].c.group_call_join(ids[4].clone()), Err(QtError::InvalidInput { .. })));
        for p in &peers[..5] {
            let parts = p.c.group_call_participants(start.call_id.clone()).unwrap();
            assert_eq!(parts.len(), 4, "cada um ve os outros 4");
            assert!(parts.iter().all(|x| x.connected), "todas as pernas com chaves");
        }
        // Saem todos: ao sexto, que ainda estava a tocar, o toque para.
        for (p, id) in peers[..5].iter().zip(std::iter::once(&start.call_id).chain(&ids)) {
            p.c.group_call_leave(id.clone()).unwrap();
            flush_all(&all);
        }
        let ev = peers[5].c.group_call_events().unwrap();
        assert!(ev.iter().any(|e| matches!(e, GroupCallEvent::Ended { reason: GroupCallEndReason::Missed, .. })), "{ev:?}");
    }

    /// C desaparece a meio (sem rede, sem bateria, app fechada a forca): o "sai" nunca chega. A e
    /// B deixam de o ter na chamada ao fim do prazo, e a perna entre A e B fica (a voz continua).
    #[test]
    fn quem_desaparece_sem_avisar_sai_da_chamada() {
        let (peers, gid) = group(3);
        let all: Vec<&Peer> = peers.iter().collect();
        let start = peers[0].c.group_call_start(gid, false).unwrap();
        let call = start.call_id.clone();
        let mut auds = vec![audio(&peers[0], &call)];
        flush_all(&all);
        for p in &peers[1..] {
            let id = incoming(p).unwrap();
            p.c.group_call_join(id.clone()).unwrap();
            auds.push(audio(p, &id));
            flush_all(&all);
        }
        let ok = run(&all, 20, || {
            peers.iter().zip(&auds).all(|(p, a)| {
                let parts = p.c.group_call_participants(call.clone()).unwrap();
                parts.len() == 2 && parts.iter().all(|x| x.connected && a.direct(x.contact_id.clone()))
            })
        });
        assert!(ok, "malha de 3 com ligacao direta em todas as pernas");
        // Os tres a falar um pouco (como na app, onde o microfone nunca para).
        let talk = |who: &[Arc<GroupAudio>], ps: &[&Peer], i: usize| {
            for a in who {
                a.push_capture(tone(i)).unwrap();
                let _ = a.pull_playback();
            }
            for p in ps {
                p.c.group_call_events().unwrap();
            }
            flush_all(ps);
            std::thread::sleep(Duration::from_millis(20));
        };
        for i in 0..50 {
            talk(&auds, &all, i);
        }
        for p in &peers[..2] {
            crate::client::relock(&p.c.group_calls).leg_timeout = Some(2_000);
        }
        // C desaparece: o motor dele para e o "sai" nunca e enviado.
        auds[2].stop();
        let t = Instant::now();
        let mut i = 0;
        let gone = loop {
            talk(&auds[..2], &all[..2], i);
            i += 1;
            let only_ab = peers[..2].iter().all(|p| {
                let parts = p.c.group_call_participants(call.clone()).unwrap();
                parts.len() == 1 && parts[0].connected
            });
            if only_ab || t.elapsed() > Duration::from_secs(10) {
                break only_ab;
            }
        };
        assert!(gone, "A e B deixam de ter C e continuam ligados um ao outro");
        assert!(t.elapsed() >= Duration::from_secs(2), "so depois do prazo");
        // A perna de C saiu tambem do audio do grupo; a de B continua.
        assert!(auds[0].diagnostic_of(peers[2].cid()).is_empty());
        assert!(!auds[0].diagnostic_of(peers[1].cid()).is_empty());
        let ev = peers[0].c.group_call_events().unwrap();
        assert!(!ev.iter().any(|e| matches!(e, GroupCallEvent::Ended { .. })), "a chamada continua para A: {ev:?}");
        for p in &peers[..2] {
            p.c.group_call_leave(call.clone()).unwrap();
        }
    }

    /// O nucleo fecha a meio da chamada de grupo (a app fecha a base de dados, por exemplo ao ser
    /// fechada nos recentes) sem a app parar a captura: o microfone deixa de chegar aos outros.
    #[test]
    fn fechar_o_nucleo_para_o_microfone() {
        let (mut peers, gid) = group(2);
        let start = peers[0].c.group_call_start(gid, false).unwrap();
        let call = start.call_id.clone();
        let a0 = audio(&peers[0], &call);
        flush_all(&peers.iter().collect::<Vec<_>>());
        let id = incoming(&peers[1]).unwrap();
        peers[1].c.group_call_join(id.clone()).unwrap();
        let a1 = audio(&peers[1], &id);
        let ok = run(&peers.iter().collect::<Vec<_>>(), 20, || {
            let parts = peers[0].c.group_call_participants(call.clone()).unwrap();
            parts.len() == 1 && parts[0].connected && a0.direct(parts[0].contact_id.clone())
        });
        assert!(ok, "A e B ligados pelo direto");
        // B fala e A ouve (`ms` de audio; devolve os frames com som do lado de A).
        let speak = |from: usize, ms: u64| {
            let t = Instant::now();
            let mut loud = 0;
            while t.elapsed() < Duration::from_millis(ms) {
                a1.push_capture(tone(from)).unwrap();
                a0.push_capture(vec![0; 1920]).unwrap();
                if rms(&a0.pull_playback()) > 1000.0 {
                    loud += 1;
                }
                let _ = a1.pull_playback();
                std::thread::sleep(Duration::from_millis(20));
            }
            loud
        };
        // Pacotes que chegaram a A vindos de B.
        let b_cid = peers[1].cid();
        let from_b = || a0.leg(&b_cid).and_then(|l| l.heard()).unwrap_or(0);
        assert!(speak(0, 2_000) > 30, "A ouve B");
        assert!(from_b() > 50);
        // O nucleo de B fecha; a app (aqui o teste) continua a dar o microfone ao audio do grupo.
        drop(peers.pop());
        let _ = speak(100, 300);
        let before = from_b();
        let _ = speak(200, 2_000);
        assert_eq!(from_b(), before, "nada do microfone de B chega a A");
        assert!(!a1.video_push(vec![7; 1500], true, 0, 0), "nem imagens");
        peers[0].c.group_call_leave(call).unwrap();
    }

    #[test]
    fn quem_recusou_pode_entrar_depois() {
        let (peers, gid) = group(3);
        let all: Vec<&Peer> = peers.iter().collect();
        let start = peers[0].c.group_call_start(gid.clone(), false).unwrap();
        flush_all(&all);
        let id1 = incoming(&peers[1]).unwrap();
        let id2 = incoming(&peers[2]).unwrap();
        peers[1].c.group_call_join(id1).unwrap();
        // C recusa: deixa de tocar, mas a chamada continua e aparece "Entrar na chamada".
        peers[2].c.group_call_leave(id2.clone()).unwrap();
        flush_all(&all);
        run(&all, 1, || false);
        assert_eq!(peers[2].c.group_call_current().unwrap(), None);
        assert_eq!(peers[2].c.group_call_ongoing(gid.clone()).unwrap(), Some(start.call_id.clone()));
        assert!(incoming(&peers[2]).is_none(), "o \"entrei\" de B nao volta a tocar");
        // C entra depois: fica com as duas pernas ligadas.
        peers[2].c.group_call_rejoin(start.call_id.clone()).unwrap();
        assert!(run(&all, 5, || peers[2].c.group_call_participants(start.call_id.clone()).unwrap().iter().filter(|p| p.connected).count() == 2));
        assert_eq!(peers[2].c.group_call_ongoing(gid.clone()).unwrap(), None);
        // Saem todos: a chamada deixa de estar "a decorrer" para quem nao esta nela.
        for p in &peers {
            p.c.group_call_leave(start.call_id.clone()).unwrap();
            flush_all(&all);
        }
    }

    /// B e C nao sao contactos e nunca escreveram no grupo: nao ha sessao entre eles. B entra
    /// antes de haver (o "entrei" dele nao chega a C); C cria a sessao ao entrar (como a app) e
    /// mesmo assim os dois ficam ligados (0.9.67), seja qual for o id menor.
    #[test]
    fn membros_sem_sessao_entre_si_ligam_se() {
        for _ in 0..2 {
            let peers: Vec<Peer> = (0..3).map(|i| Peer::new(&format!("s{i}"))).collect();
            connect(&peers[0], &peers[1]);
            connect(&peers[0], &peers[2]);
            let members = peers[1..].iter().map(|p| p.id.public_id.clone()).collect();
            let gid = peers[0].c.group_create("Amigos".into(), members).unwrap().group_id;
            let all: Vec<&Peer> = peers.iter().collect();
            flush_all(&all);
            for p in &peers {
                p.c.group_call_accept_notice().unwrap();
            }
            let start = peers[0].c.group_call_start(gid, false).unwrap();
            flush_all(&all);
            let (id1, id2) = (incoming(&peers[1]).unwrap(), incoming(&peers[2]).unwrap());
            peers[1].c.group_call_join(id1).unwrap();
            flush_all(&all);
            // C cria a sessao com B ao entrar (a app faz `ensureSessions` antes de entrar).
            connect(&peers[2], &peers[1]);
            peers[2].c.group_call_join(id2).unwrap();
            let ok = run(&all, 5, || {
                peers.iter().all(|p| p.c.group_call_participants(start.call_id.clone()).unwrap().iter().filter(|x| x.connected).count() == 2)
            });
            assert!(ok, "todos ligados a todos");
            for p in &peers {
                p.c.group_call_leave(start.call_id.clone()).unwrap();
            }
        }
    }

    #[test]
    fn quem_nao_e_do_grupo_nao_entra() {
        let (peers, gid) = group(2);
        let intruso = Peer::new("intruso");
        connect(&intruso, &peers[1]);
        intruso.c.group_call_accept_notice().unwrap();
        // O intruso nao faz parte do grupo: nao consegue ligar-lhe.
        assert!(intruso.c.group_call_start(gid, false).is_err());
        assert!(incoming(&peers[1]).is_none());
    }
}
