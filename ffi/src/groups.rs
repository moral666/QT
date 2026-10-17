//! Grupos ate 10 pessoas, com no maximo 3 administradores.
//!
//! DESENHO: envio a cada membro. Cada mensagem de grupo e cifrada uma vez para
//! cada membro, pelas sessoes 1 a 1 que ja existem (PQXDH + Double Ratchet +
//! Sealed Sender). O relay nao muda e nem sabe que existe um grupo: so ve
//! envelopes 1 a 1 como os outros. Com 10 pessoas, sao no maximo 9 envios.
//!
//! FORMATO (dentro do Double Ratchet, cabecalho v1 de `history.rs`):
//!   tipo 6 = mensagem de grupo: [16 group_id][mensagem v1 completa: texto ou reacao]
//!   tipo 7 = info do grupo:     [16 group_id][JSON: versao, nome, membros]
//!   tipo 8 = saida do grupo:    [16 group_id]
//!
//! REGRAS
//!   * So um administrador muda o grupo (nome, membros, administradores), e cada
//!     mudanca sobe a versao: uma info com versao igual ou mais antiga e ignorada.
//!   * Uma info de um grupo que ainda nao conheco so e aceite se vier de um
//!     administrador que a propria info nomeia e se eu estiver nos membros. Num
//!     grupo que ja conheco, tem de vir de quem ja era administrador.
//!   * Uma mensagem so e aceite de quem esta nos membros. Se chegar antes da info
//!     do grupo (os envelopes de pessoas diferentes nao chegam por ordem), fica
//!     guardada ate a info chegar (no maximo `MAX_PENDING` por telemovel).
//!   * Quem nao e administrador sai com o tipo 8; os administradores que a
//!     recebem publicam logo uma versao nova sem essa pessoa.
//!   * Sem recibos de entregue/visto nos grupos (menos trafego); a mensagem passa
//!     a "Enviada" quando o relay confirmou o envelope de todos os membros.
//!
//! Conversa do grupo no historico: `contact_id` = "g:" + group_id (hex), e cada
//! mensagem recebida guarda quem a escreveu em `sender`.

use serde::{Deserialize, Serialize};

use crate::client::{bad, hex, parse_public_id, public_identity, unhex, QtClient, QtError};
use crate::history::{
    encode_control, encode_text, is_blocked_in, new_id, now_ms, to_chat, with_reactions,
    ChatMessage, ReceiveResult, HDR, PAYLOAD_V1, T_GROUP_INFO, T_GROUP_LEAVE, T_GROUP_MSG, T_REACTION, T_TEXT,
    UNSUPPORTED_TEXT,
};
use qt_storage as st;
use qt_storage::messages as hm;

pub const MAX_GROUP_MEMBERS: usize = 10;
pub const MAX_GROUP_ADMINS: usize = 3;
/// Nome maximo do grupo (bytes UTF-8).
pub const MAX_GROUP_NAME: usize = 100;
/// Mensagens de grupos ainda desconhecidos guardadas a espera da info.
const MAX_PENDING: usize = 100;
/// Prefixo da conversa de um grupo no historico.
pub const GROUP_CONV_PREFIX: &str = "g:";
const KV_PENDING: &str = "gpend:";
const KV_OUT: &str = "gout:";
const KV_COUNT: &str = "gcnt:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredMember {
    pub pid: String,
    pub admin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredGroup {
    pub v: u32,
    pub name: String,
    pub members: Vec<StoredMember>,
}

impl StoredGroup {
    fn valid(&self) -> bool {
        let admins = self.members.iter().filter(|m| m.admin).count();
        if self.name.trim().is_empty() || self.name.len() > MAX_GROUP_NAME || self.name.contains(['\n', '\r']) {
            return false;
        }
        if self.members.is_empty() || self.members.len() > MAX_GROUP_MEMBERS || admins == 0 || admins > MAX_GROUP_ADMINS {
            return false;
        }
        let mut seen = std::collections::HashSet::new();
        self.members.iter().all(|m| member_cid(&m.pid).is_some_and(|c| seen.insert(c)))
    }

    fn has(&self, cid: &str) -> bool {
        self.members.iter().any(|m| member_cid(&m.pid).as_deref() == Some(cid))
    }

    fn is_admin(&self, cid: &str) -> bool {
        self.members.iter().any(|m| m.admin && member_cid(&m.pid).as_deref() == Some(cid))
    }

    fn without(&self, cid: &str) -> StoredGroup {
        StoredGroup {
            v: self.v,
            name: self.name.clone(),
            members: self.members.iter().filter(|m| member_cid(&m.pid).as_deref() != Some(cid)).cloned().collect(),
        }
    }

    /// Sem administradores (o ultimo saiu): o primeiro membro passa a administrador.
    fn ensure_admin(&mut self) {
        if !self.members.is_empty() && !self.members.iter().any(|m| m.admin) {
            self.members[0].admin = true;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GroupMember {
    pub public_id: String,
    pub contact_id: String,
    pub admin: bool,
    /// Sou eu.
    pub me: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GroupInfo {
    pub group_id: String,
    /// Id da conversa no historico ("g:" + group_id): usar em `messages_page`, etc.
    pub conversation_id: String,
    pub name: String,
    pub members: Vec<GroupMember>,
    pub i_am_admin: bool,
    /// false: sai do grupo ou fui removido (o historico fica ate ser apagado).
    pub active: bool,
    pub version: u32,
}

/// Para criar ou mudar um grupo: um membro e se e administrador.
#[derive(Debug, Clone, uniffi::Record)]
pub struct GroupMemberInput {
    pub public_id: String,
    pub admin: bool,
}

pub(crate) fn member_cid(pid: &str) -> Option<String> {
    parse_public_id(pid).ok().map(|(id, _)| hex(&id))
}

fn conv_of(gid: &str) -> String {
    format!("{GROUP_CONV_PREFIX}{gid}")
}

fn gid_from_conv(conv: &str) -> Option<&str> {
    conv.strip_prefix(GROUP_CONV_PREFIX)
}

fn gid_hex(b: &[u8]) -> Option<String> {
    (b.len() >= 16).then(|| hex(&b[..16]))
}

fn parse_gid(group_id: &str) -> Result<Vec<u8>, QtError> {
    match unhex(group_id) {
        Some(b) if b.len() == 16 => Ok(b),
        _ => Err(bad("group_id")),
    }
}

fn my_identity(c: &st::rusqlite::Connection) -> Result<(String, String), QtError> {
    let p = public_identity(&st::load_identity(c)?);
    Ok((hex(&p.identity_public), p.public_id))
}

fn load(c: &st::rusqlite::Connection, gid: &str) -> Result<Option<(StoredGroup, bool)>, QtError> {
    Ok(match hm::group_get(c, gid)? {
        Some((json, active)) => serde_json::from_str(&json).ok().map(|g| (g, active)),
        None => None,
    })
}

fn save(c: &st::rusqlite::Connection, gid: &str, g: &StoredGroup, active: bool) -> Result<(), QtError> {
    let json = serde_json::to_string(g).map_err(|_| QtError::Storage)?;
    hm::group_put(c, gid, &json, active)?;
    Ok(())
}

fn to_info(gid: &str, g: &StoredGroup, active: bool, me: &str) -> GroupInfo {
    GroupInfo {
        group_id: gid.to_string(),
        conversation_id: conv_of(gid),
        name: g.name.clone(),
        members: g
            .members
            .iter()
            .filter_map(|m| {
                let cid = member_cid(&m.pid)?;
                Some(GroupMember { public_id: m.pid.clone(), me: cid == me, contact_id: cid, admin: m.admin })
            })
            .collect(),
        i_am_admin: active && g.is_admin(me),
        active,
        version: g.v,
    }
}

/// Cifra `kind`+`body` para cada destinatario (Public IDs) e poe na outbox.
/// Salta-se a mim, os bloqueados e quem ainda nao tem sessao (devolvidos em
/// `missing`: a app tem de abrir as sessoes antes de enviar). Com `msg_id`, conta
/// os envios para marcar a mensagem como "Enviada" quando o relay confirmar todos.
fn fanout(
    client: &QtClient,
    c: &st::rusqlite::Connection,
    recipients: &[String],
    kind: u8,
    body: &[u8],
    msg_id: Option<&[u8]>,
) -> Result<Vec<String>, QtError> {
    let (me, _) = my_identity(c)?;
    let mut missing = Vec::new();
    let mut sent = 0u32;
    let mut done = std::collections::HashSet::new();
    // Guardado por envelope ate o relay o confirmar: [1 tipo][1 tem msg_id][16 msg_id?][corpo].
    // Se a sessao com esse membro for substituida antes do envio, volta a ser cifrado
    // com a sessao nova (`requeue_outbox_of`) em vez de se perder.
    let mut pending = vec![kind, msg_id.is_some() as u8];
    if let Some(mid) = msg_id {
        pending.extend_from_slice(mid);
    }
    pending.extend_from_slice(body);
    for pid in recipients {
        let Some(cid) = member_cid(pid) else { continue };
        if cid == me || !done.insert(cid.clone()) || is_blocked_in(c, &cid)? {
            continue;
        }
        if st::load_session(c, &cid)?.is_none() {
            missing.push(pid.clone());
            continue;
        }
        put_envelope(client, c, &cid, &pending)?;
        sent += 1;
    }
    if let Some(mid) = msg_id {
        if sent == 0 {
            hm::mark_sent(c, mid)?;
        } else {
            st::kv_set(c, &format!("{KV_COUNT}{}", hex(mid)), &sent.to_be_bytes())?;
        }
    }
    Ok(missing)
}

/// Cifra um envelope de grupo guardado (`pending`, ver `fanout`) para `cid` e poe-no na outbox.
fn put_envelope(client: &QtClient, c: &st::rusqlite::Connection, cid: &str, pending: &[u8]) -> Result<(), QtError> {
    let (kind, body) = split_pending(pending).ok_or(QtError::Storage)?;
    // Sem convite ("Quer falar contigo"): estar no mesmo grupo nao e pedir conversa
    // a dois. O convite segue com a primeira mensagem a dois, se houver.
    let blob = client.seal_with(c, cid, &encode_control(kind, body)?)?;
    let out = new_id()?;
    hm::outbox_put(c, &out, cid, hm::OUTBOX_GROUP, &blob)?;
    st::kv_set(c, &format!("{KV_OUT}{}", hex(&out)), pending)?;
    Ok(())
}

/// (tipo, corpo) de um envelope guardado.
fn split_pending(p: &[u8]) -> Option<(u8, &[u8])> {
    match p {
        [kind, 0, body @ ..] => Some((*kind, body)),
        [kind, 1, rest @ ..] if rest.len() >= 16 => Some((*kind, &rest[16..])),
        _ => None,
    }
}

/// msg_id da mensagem de grupo deste envelope guardado (None nas reacoes e infos).
fn pending_msg_id(p: &[u8]) -> Option<&[u8]> {
    match p {
        [_, 1, rest @ ..] if rest.len() >= 16 => Some(&rest[..16]),
        _ => None,
    }
}

/// O relay confirmou um envelope de grupo (`outbox_sent`).
pub(crate) fn on_outbox_sent(c: &st::rusqlite::Connection, outbox_id: &[u8]) -> Result<(), QtError> {
    let key = format!("{KV_OUT}{}", hex(outbox_id));
    let Some(pending) = st::kv_get(c, &key)? else { return Ok(()) };
    st::kv_delete(c, &key)?;
    let Some(mid) = pending_msg_id(&pending).map(|m| m.to_vec()) else { return Ok(()) };
    let ckey = format!("{KV_COUNT}{}", hex(&mid));
    let left = st::kv_get(c, &ckey)?
        .and_then(|v| <[u8; 4]>::try_from(v.as_slice()).ok())
        .map(u32::from_be_bytes)
        .unwrap_or(1)
        .saturating_sub(1);
    if left == 0 {
        st::kv_delete(c, &ckey)?;
        hm::mark_sent(c, &mid)?;
    } else {
        st::kv_set(c, &ckey, &left.to_be_bytes())?;
    }
    Ok(())
}

/// A sessao com `contact_id` foi substituida (a nova ja esta guardada) e a outbox
/// dele foi cifrada com a antiga: os envelopes de grupo que la estavam voltam a
/// ser cifrados com a sessao nova, pela mesma ordem. Chamar ANTES de apagar a outbox.
pub(crate) fn requeue_outbox_of(client: &QtClient, c: &st::rusqlite::Connection, contact_id: &str) -> Result<(), QtError> {
    let old: Vec<Vec<u8>> = hm::outbox_all(c)?
        .into_iter()
        .filter(|e| e.contact_id == contact_id && e.kind == hm::OUTBOX_GROUP)
        .map(|e| e.outbox_id)
        .collect();
    for id in old {
        let key = format!("{KV_OUT}{}", hex(&id));
        let pending = st::kv_get(c, &key)?;
        st::kv_delete(c, &key)?;
        hm::outbox_take(c, &id)?;
        if let Some(p) = pending {
            put_envelope(client, c, contact_id, &p)?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ receber

/// Tipo 6: mensagem de um membro para o grupo.
pub(crate) fn on_group_msg(c: &st::rusqlite::Connection, from: &str, body: &[u8]) -> Result<ReceiveResult, QtError> {
    let Some(gid) = gid_hex(body) else { return Ok(no_change(from)) };
    match load(c, &gid)? {
        None => {
            // A info do grupo ainda nao chegou: guarda ate chegar.
            let n: i64 = c.query_row("SELECT COUNT(*) FROM kv WHERE k LIKE ?1", [format!("{KV_PENDING}%")], |r| r.get(0))?;
            if (n as usize) < MAX_PENDING {
                let mut v = from.as_bytes().to_vec();
                v.extend_from_slice(&body[16..]);
                st::kv_set(c, &format!("{KV_PENDING}{gid}:{}", hex(&new_id()?)), &v)?;
            }
            Ok(no_change(from))
        }
        Some((g, active)) => {
            if !active || !g.has(from) {
                return Ok(no_change(from));
            }
            apply_member_msg(c, &gid, from, &body[16..])
        }
    }
}

fn no_change(contact_id: &str) -> ReceiveResult {
    ReceiveResult::NoChange { contact_id: contact_id.to_string() }
}

/// Guarda no historico do grupo a mensagem (texto) ou a reacao de `from`.
fn apply_member_msg(c: &st::rusqlite::Connection, gid: &str, from: &str, inner: &[u8]) -> Result<ReceiveResult, QtError> {
    let conv = conv_of(gid);
    if inner.len() < HDR || inner[0] != PAYLOAD_V1 {
        return Ok(no_change(&conv));
    }
    let id = &inner[2..18];
    let sent_at = i64::from_be_bytes(inner[18..26].try_into().map_err(|_| QtError::InvalidEnvelope)?);
    let now = now_ms();
    let stored = match inner[1] {
        T_TEXT => match std::str::from_utf8(&inner[HDR..]) {
            Ok(text) if !text.trim().is_empty() => {
                hm::insert_message_from(c, id, &conv, false, hm::KIND_TEXT, text, sent_at, now, hm::ST_RECEIVED, from)?
            }
            _ => return Ok(no_change(&conv)),
        },
        T_REACTION => {
            let b = &inner[HDR..];
            if b.len() < 16 || b.len() > 16 + 32 {
                return Ok(no_change(&conv));
            }
            let Ok(emoji) = std::str::from_utf8(&b[16..]) else { return Ok(no_change(&conv)) };
            if emoji.contains(['\n', '\r']) || hm::get_message(c, &conv, &b[..16])?.is_none() {
                return Ok(no_change(&conv));
            }
            hm::set_reaction(c, &conv, &b[..16], from, emoji)?;
            return Ok(ReceiveResult::Updated { contact_id: conv });
        }
        // Imagens, voz e outros tipos ainda nao vao para grupos nesta versao.
        _ => hm::insert_message_from(c, id, &conv, false, hm::KIND_TEXT, UNSUPPORTED_TEXT, sent_at, now, hm::ST_RECEIVED, from)?,
    };
    Ok(match stored {
        Some(_) => match hm::get_message(c, &conv, id)? {
            Some(m) => ReceiveResult::NewMessage { message: to_chat(&m) },
            None => no_change(&conv),
        },
        None => no_change(&conv),
    })
}

/// Tipo 7: um administrador criou ou mudou o grupo.
pub(crate) fn on_group_info(c: &st::rusqlite::Connection, from: &str, body: &[u8]) -> Result<ReceiveResult, QtError> {
    let Some(gid) = gid_hex(body) else { return Ok(no_change(from)) };
    let Ok(g) = serde_json::from_slice::<StoredGroup>(&body[16..]) else { return Ok(no_change(from)) };
    if !g.valid() {
        return Ok(no_change(from));
    }
    let (me, _) = my_identity(c)?;
    match load(c, &gid)? {
        None => {
            if !g.is_admin(from) || !g.has(&me) {
                return Ok(no_change(from));
            }
            save(c, &gid, &g, true)?;
            replay_pending(c, &gid)?;
        }
        Some((old, _)) => {
            // So um administrador atual muda o grupo, e so para uma versao mais recente.
            // (Pode ser uma versao sem ele: um administrador que sai do grupo.)
            if !old.is_admin(from) || g.v <= old.v {
                return Ok(no_change(from));
            }
            save(c, &gid, &g, g.has(&me))?;
        }
    }
    Ok(ReceiveResult::Updated { contact_id: conv_of(&gid) })
}

/// Mensagens que chegaram antes da info do grupo: agora que o grupo existe.
fn replay_pending(c: &st::rusqlite::Connection, gid: &str) -> Result<(), QtError> {
    let prefix = format!("{KV_PENDING}{gid}:");
    let rows: Vec<(String, Vec<u8>)> = {
        let mut stmt = c.prepare("SELECT k, v FROM kv WHERE k LIKE ?1 ORDER BY k")?;
        let r = stmt.query_map([format!("{prefix}%")], |r| Ok((r.get(0)?, r.get(1)?)))?;
        r.collect::<Result<Vec<_>, _>>()?
    };
    let group = load(c, gid)?;
    for (k, v) in rows {
        st::kv_delete(c, &k)?;
        if v.len() < 64 {
            continue;
        }
        let from = String::from_utf8_lossy(&v[..64]).to_string();
        if let Some((g, true)) = &group {
            if g.has(&from) && !is_blocked_in(c, &from)? {
                apply_member_msg(c, gid, &from, &v[64..])?;
            }
        }
    }
    Ok(())
}

/// Tipo 8: um membro saiu. Os administradores publicam logo uma versao sem ele.
pub(crate) fn on_group_leave(client: &QtClient, c: &st::rusqlite::Connection, from: &str, body: &[u8]) -> Result<ReceiveResult, QtError> {
    let Some(gid) = gid_hex(body) else { return Ok(no_change(from)) };
    let Some((g, active)) = load(c, &gid)? else { return Ok(no_change(from)) };
    if !active || !g.has(from) {
        return Ok(no_change(from));
    }
    let (me, _) = my_identity(c)?;
    let mut ng = g.without(from);
    if ng.is_admin(&me) || (!ng.members.iter().any(|m| m.admin) && ng.members.first().and_then(|m| member_cid(&m.pid)).as_deref() == Some(&me)) {
        ng.ensure_admin();
        ng.v = g.v + 1;
        save(c, &gid, &ng, true)?;
        let others: Vec<String> = ng.members.iter().map(|m| m.pid.clone()).collect();
        let mut body = parse_gid(&gid)?;
        body.extend_from_slice(&serde_json::to_vec(&ng).map_err(|_| QtError::Storage)?);
        fanout(client, c, &others, T_GROUP_INFO, &body, None)?;
    } else {
        save(c, &gid, &ng, true)?;
    }
    Ok(ReceiveResult::Updated { contact_id: conv_of(&gid) })
}

// ------------------------------------------------------------------ API

#[uniffi::export]
impl QtClient {
    /// CRIAR GRUPO: eu (administrador) + ate 9 pessoas. Abrir antes as sessoes com
    /// todas (`start_session`): quem nao tiver sessao nao recebe a info do grupo.
    pub fn group_create(&self, name: String, member_public_ids: Vec<String>) -> Result<GroupInfo, QtError> {
        let c = self.lock()?;
        let (me, my_pid) = my_identity(&c)?;
        let mut members = vec![StoredMember { pid: my_pid, admin: true }];
        for pid in member_public_ids {
            if member_cid(&pid).ok_or_else(|| bad("public_id"))? != me {
                members.push(StoredMember { pid, admin: false });
            }
        }
        let g = StoredGroup { v: 1, name: name.trim().to_string(), members };
        if !g.valid() {
            return Err(bad("group"));
        }
        let gid = hex(&new_id()?);
        let tx = c.unchecked_transaction()?;
        save(&tx, &gid, &g, true)?;
        let mut body = parse_gid(&gid)?;
        body.extend_from_slice(&serde_json::to_vec(&g).map_err(|_| QtError::Storage)?);
        let recipients: Vec<String> = g.members.iter().map(|m| m.pid.clone()).collect();
        fanout(self, &tx, &recipients, T_GROUP_INFO, &body, None)?;
        tx.commit()?;
        Ok(to_info(&gid, &g, true, &me))
    }

    /// Todos os grupos (tambem os de que sai: `active` = false).
    pub fn groups(&self) -> Result<Vec<GroupInfo>, QtError> {
        let c = self.lock()?;
        let (me, _) = my_identity(&c)?;
        Ok(hm::groups_all(&c)?
            .into_iter()
            .filter_map(|(gid, json, active)| serde_json::from_str::<StoredGroup>(&json).ok().map(|g| to_info(&gid, &g, active, &me)))
            .collect())
    }

    pub fn group(&self, group_id: String) -> Result<Option<GroupInfo>, QtError> {
        let c = self.lock()?;
        let (me, _) = my_identity(&c)?;
        Ok(load(&c, &group_id)?.map(|(g, active)| to_info(&group_id, &g, active, &me)))
    }

    /// MUDAR O GRUPO (so administradores): nome, membros e administradores. Tenho
    /// de continuar nos membros (para sair, `group_leave`). Quem sai da lista
    /// tambem recebe a info nova (fica a saber que foi removido).
    pub fn group_update(&self, group_id: String, name: String, members: Vec<GroupMemberInput>) -> Result<GroupInfo, QtError> {
        let c = self.lock()?;
        let (me, _) = my_identity(&c)?;
        let (old, active) = load(&c, &group_id)?.ok_or_else(|| bad("group_id"))?;
        if !active || !old.is_admin(&me) {
            return Err(bad("not_admin"));
        }
        let g = StoredGroup {
            v: old.v + 1,
            name: name.trim().to_string(),
            members: members.into_iter().map(|m| StoredMember { pid: m.public_id, admin: m.admin }).collect(),
        };
        if !g.valid() || !g.has(&me) {
            return Err(bad("group"));
        }
        let tx = c.unchecked_transaction()?;
        save(&tx, &group_id, &g, true)?;
        let mut body = parse_gid(&group_id)?;
        body.extend_from_slice(&serde_json::to_vec(&g).map_err(|_| QtError::Storage)?);
        let mut recipients: Vec<String> = g.members.iter().map(|m| m.pid.clone()).collect();
        recipients.extend(old.members.iter().map(|m| m.pid.clone()));
        fanout(self, &tx, &recipients, T_GROUP_INFO, &body, None)?;
        tx.commit()?;
        Ok(to_info(&group_id, &g, true, &me))
    }

    /// SAIR DO GRUPO. Um administrador publica uma versao sem ele (se era o unico
    /// administrador, o primeiro membro que fica passa a se-lo); os outros avisam
    /// que sairam. O historico fica ate ser apagado (`delete_conversation`).
    pub fn group_leave(&self, group_id: String) -> Result<(), QtError> {
        let c = self.lock()?;
        let (me, _) = my_identity(&c)?;
        let Some((g, active)) = load(&c, &group_id)? else { return Ok(()) };
        if !active {
            return Ok(());
        }
        let others: Vec<String> = g.without(&me).members.iter().map(|m| m.pid.clone()).collect();
        let tx = c.unchecked_transaction()?;
        if g.is_admin(&me) {
            let mut ng = g.without(&me);
            ng.ensure_admin();
            ng.v = g.v + 1;
            let mut body = parse_gid(&group_id)?;
            body.extend_from_slice(&serde_json::to_vec(&ng).map_err(|_| QtError::Storage)?);
            fanout(self, &tx, &others, T_GROUP_INFO, &body, None)?;
            save(&tx, &group_id, &ng, false)?;
        } else {
            fanout(self, &tx, &others, T_GROUP_LEAVE, &parse_gid(&group_id)?, None)?;
            save(&tx, &group_id, &g, false)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// APAGAR GRUPO deste telemovel: sai (se ainda estiver), apaga o historico e o grupo.
    pub fn group_delete(&self, group_id: String) -> Result<(), QtError> {
        self.group_leave(group_id.clone())?;
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        hm::delete_conversation(&tx, &conv_of(&group_id))?;
        hm::group_delete(&tx, &group_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Public IDs dos membros com quem ainda nao ha sessao (a app abre-as antes de enviar).
    pub fn group_members_needing_session(&self, group_id: String) -> Result<Vec<String>, QtError> {
        let c = self.lock()?;
        let (me, _) = my_identity(&c)?;
        let Some((g, _)) = load(&c, &group_id)? else { return Ok(vec![]) };
        let mut out = Vec::new();
        for m in &g.members {
            let Some(cid) = member_cid(&m.pid) else { continue };
            if cid != me && !is_blocked_in(&c, &cid)? && st::load_session(&c, &cid)?.is_none() {
                out.push(m.pid.clone());
            }
        }
        Ok(out)
    }

    /// ENVIAR TEXTO ao grupo: guardado no historico do grupo e um envelope por membro.
    pub fn group_send_text(&self, group_id: String, text: String) -> Result<ChatMessage, QtError> {
        if text.trim().is_empty() || text.len() > crate::history::MAX_TEXT_BYTES {
            return Err(bad("text"));
        }
        let gid = parse_gid(&group_id)?;
        let conv = conv_of(&group_id);
        let c = self.lock()?;
        let (g, active) = load(&c, &group_id)?.ok_or_else(|| bad("group_id"))?;
        if !active {
            return Err(bad("group_inactive"));
        }
        let id = new_id()?;
        let now = now_ms();
        let tx = c.unchecked_transaction()?;
        hm::insert_message(&tx, &id, &conv, true, hm::KIND_TEXT, text.trim(), now, now, hm::ST_PENDING)?;
        let mut body = gid;
        body.extend_from_slice(&encode_text(&id, now, text.trim()));
        let recipients: Vec<String> = g.members.iter().map(|m| m.pid.clone()).collect();
        fanout(self, &tx, &recipients, T_GROUP_MSG, &body, Some(&id))?;
        let m = hm::get_message(&tx, &conv, &id)?.ok_or(QtError::Storage)?;
        tx.commit()?;
        Ok(to_chat(&m))
    }

    /// REAGIR a uma mensagem do grupo (emoji vazio retira a minha reacao).
    pub fn group_react(&self, group_id: String, message_id: String, emoji: String) -> Result<(), QtError> {
        let gid = parse_gid(&group_id)?;
        let target = match unhex(&message_id) {
            Some(b) if b.len() == 16 => b,
            _ => return Err(bad("message_id")),
        };
        if emoji.len() > 32 || emoji.contains(['\n', '\r']) {
            return Err(bad("emoji"));
        }
        let conv = conv_of(&group_id);
        let c = self.lock()?;
        let (g, active) = load(&c, &group_id)?.ok_or_else(|| bad("group_id"))?;
        if !active || hm::get_message(&c, &conv, &target)?.is_none() {
            return Err(bad("message_id"));
        }
        let tx = c.unchecked_transaction()?;
        hm::set_reaction(&tx, &conv, &target, "", &emoji)?;
        let mut inner_body = target.clone();
        inner_body.extend_from_slice(emoji.as_bytes());
        let mut body = gid;
        body.extend_from_slice(&encode_control(T_REACTION, &inner_body)?);
        let recipients: Vec<String> = g.members.iter().map(|m| m.pid.clone()).collect();
        fanout(self, &tx, &recipients, T_GROUP_MSG, &body, None)?;
        tx.commit()?;
        Ok(())
    }

    /// Pagina do historico de um grupo (atalho para `messages_page` com o id da conversa).
    pub fn group_messages_page(&self, group_id: String, before_seq: Option<i64>, limit: u32) -> Result<Vec<ChatMessage>, QtError> {
        let conv = conv_of(&group_id);
        let c = self.lock()?;
        let msgs = hm::messages_page(&c, &conv, before_seq, limit)?.iter().map(to_chat).collect();
        with_reactions(&c, &conv, msgs)
    }
}

/// O id do grupo de uma conversa ("g:..."), se for de um grupo.
#[uniffi::export]
pub fn group_id_of_conversation(conversation_id: String) -> Option<String> {
    gid_from_conv(&conversation_id).map(|s| s.to_string())
}
