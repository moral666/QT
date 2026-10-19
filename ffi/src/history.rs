//! Historico de conversas, estados das mensagens e fila de saida - API para a app.
//!
//! FORMATO DA MENSAGEM (v1) - vai DENTRO do Double Ratchet (o relay nunca o ve):
//!   [1 versao=0x01][1 tipo][16 msg_id][8 sent_at ms, BE]
//!   tipo 1 = texto:  [texto UTF-8]
//!   tipo 2 = recibo: [1 estado: 2=entregue, 3=vista][2 n BE][n x 16 msg_id]
//!   tipo 3 = anexo:  imagem/GIF (chave do ficheiro, id no relay, tamanho, tipo);
//!                    ver `attachments.rs`
//!   tipo 4 = convite: [Public ID de quem envia, "qt1:..."]. Vai antes da 1.a
//!                    mensagem de quem inicia a conversa; nao aparece no historico.
//!   tipo 5 = reacao: [16 msg_id da mensagem][emoji UTF-8; vazio = retirar]
//!   tipos 6 a 8 = grupos (mensagem, info, saida): ver `groups.rs`
//!   tipo 11 = chamada de voz (oferta, a tocar, resposta, desligar): ver `call.rs`
//! Um plaintext que nao siga este formato (clientes antigos) e tratado como
//! texto simples, com um msg_id derivado do envelope (deduplicacao estavel).
//!
//! FIABILIDADE: tudo o que muda estado acontece numa unica transacao SQLCipher:
//!  * enviar  = avancar o ratchet + guardar a mensagem + pôr o envelope na outbox;
//!  * receber = avancar o ratchet + marcar o envelope como visto + guardar a
//!    mensagem (ou aplicar o recibo) + agendar o recibo "entregue".
//!
//! Se a app morrer a meio, ou fica tudo ou nao fica nada: nao ha mensagens
//! perdidas nem envelopes "vistos" sem texto. A outbox so e limpa quando o
//! relay confirma (`outbox_sent`); ate la `outbox()` devolve o envelope para
//! reenviar (o destinatario deduplica pelo msg_id).
//!
//! Os recibos sao mensagens CIFRADAS ponta a ponta: o relay nao os ve nem os
//! consegue forjar, e um contacto so consegue mudar o estado de mensagens que
//! lhe foram enviadas a ele.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::attachments::{decode_wire, encode_wire, received_body, AttMeta, AttachmentInfo};
use crate::client::{bad, contact_to_identity, hex, unhex, QtClient, QtError};
use qt_storage as st;
use qt_storage::messages as hm;
use sha2::{Digest, Sha256};

pub(crate) const PAYLOAD_V1: u8 = 0x01;
pub(crate) const T_TEXT: u8 = 1;
const T_RECEIPT: u8 = 2;
pub(crate) const T_ATTACHMENT: u8 = 3;
const T_INTRO: u8 = 4;
pub(crate) const T_REACTION: u8 = 5;
pub(crate) const T_GROUP_MSG: u8 = 6;
pub(crate) const T_GROUP_INFO: u8 = 7;
pub(crate) const T_GROUP_LEAVE: u8 = 8;
/// Tipo 10: "sessao terminada". Quem apagou o contacto avisa, com a sessao antiga, que
/// apagou as chaves: quem recebe apaga a sessao dele e a proxima mensagem abre uma nova
/// (do outro lado aparece como convite). Corpo vazio. Nunca mostrado no ecra.
pub(crate) const T_SESSION_END: u8 = 10;
/// Emoji maximo por reacao (bytes UTF-8): chega para um emoji composto (familia, bandeira).
const MAX_EMOJI_BYTES: usize = 32;
pub(crate) const HDR: usize = 1 + 1 + 16 + 8;
/// Texto maximo por mensagem (bytes UTF-8). Cabe no limite do envelope.
pub const MAX_TEXT_BYTES: usize = 9_000;
/// Ids por recibo (16 bytes cada; 400 x 16 = 6400 bytes).
const MAX_RECEIPT_IDS: usize = 400;
const KV_PENDING_DELIVERED: &str = "rcpt_delivered:";
/// Por enviar: o convite (o meu Public ID) a este contacto, antes da proxima mensagem.
pub(crate) const KV_SEND_INTRO: &str = "send_intro:";
/// Recebido: o Public ID de quem me escreveu sem estar nos meus contactos (convite).
const KV_INTRO: &str = "intro:";
/// O contacto terminou a sessao (tipo 10): falta abrir uma nova e voltar a cifrar o
/// que estava por entregar. Enquanto isto existir, a outbox dele fica retida.
pub(crate) const KV_RESTART: &str = "restart:";
/// Endereco no relay de uma entrada da outbox para quem ja nao esta nos contactos
/// (o aviso "sessao terminada" de `delete_contact_notify`).
const KV_OUT_RELAY: &str = "out_relay:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MessageStatus {
    /// Cifrada e guardada; ainda nao confirmada pelo relay.
    Pending,
    /// O relay confirmou que a guardou ("Enviado").
    Sent,
    /// O destinatario recebeu-a ("Entregue").
    Delivered,
    /// O destinatario abriu a conversa ("Visto").
    Read,
    /// Mensagem recebida, ainda por ler.
    Received,
    /// Mensagem recebida e ja lida por mim.
    ReadByMe,
}

impl MessageStatus {
    fn from_db(v: i64) -> MessageStatus {
        match v {
            hm::ST_PENDING => MessageStatus::Pending,
            hm::ST_SENT => MessageStatus::Sent,
            hm::ST_DELIVERED => MessageStatus::Delivered,
            hm::ST_READ => MessageStatus::Read,
            hm::ST_READ_BY_ME => MessageStatus::ReadByMe,
            _ => MessageStatus::Received,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ChatMessage {
    /// Ordem local (crescente). Usar para paginar.
    pub seq: i64,
    pub message_id: String,
    pub contact_id: String,
    pub outgoing: bool,
    pub text: String,
    /// Hora de envio segundo o relogio de quem enviou (ms). Informativa.
    pub sent_at_ms: i64,
    /// Hora local em que foi criada/recebida (ms).
    pub local_at_ms: i64,
    pub status: MessageStatus,
    /// Imagem/GIF (o `text` e entao "📷 Foto" ou "GIF", para a lista de conversas).
    pub attachment: Option<AttachmentInfo>,
    /// Reacoes com emoji (vazio na lista de conversas e nas mensagens acabadas de chegar).
    pub reactions: Vec<Reaction>,
    /// Num grupo: o contact_id de quem escreveu (vazio nas conversas 1 a 1 e nas minhas).
    pub sender: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Reaction {
    pub emoji: String,
    /// A reacao e minha.
    pub mine: bool,
    /// Quem reagiu (contact_id); vazio se `mine`.
    pub from_contact: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ConversationSummary {
    pub contact_id: String,
    pub last_message: ChatMessage,
    pub unread: u32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct OutboxItem {
    pub outbox_id: String,
    pub contact_id: String,
    /// Envelope selado, pronto para `relay_request_send_item`.
    pub blob: Vec<u8>,
    /// Recibo, reacao, convite ou presenca: o relay entrega, mas nao toca a
    /// campainha (notificacao) a uma app fechada. Mensagens e grupos: `false`.
    pub silent: bool,
    /// So no aviso "sessao terminada" a quem ja nao esta nos contactos: o endereco no
    /// relay (nos outros casos, `None`: a app usa o do contacto).
    pub relay_id: Option<String>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum ReceiveResult {
    /// Mensagem nova, ja guardada no historico.
    NewMessage { message: ChatMessage },
    /// Recibo do contacto: estas mensagens enviadas mudaram de estado.
    StatusChanged { contact_id: String, message_ids: Vec<String>, status: MessageStatus },
    /// Conteudo ja conhecido (reentrega) ou recibo sem efeito: nada a mostrar.
    NoChange { contact_id: String },
    /// Algo mudou nesta conversa sem mensagem nova (ex.: uma reacao): atualizar o ecra.
    Updated { contact_id: String },
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub(crate) fn new_id() -> Result<[u8; 16], QtError> {
    let mut id = [0u8; 16];
    getrandom::getrandom(&mut id).map_err(|_| QtError::Crypto)?;
    Ok(id)
}

pub(crate) fn to_chat(m: &hm::StoredMessage) -> ChatMessage {
    let att = if m.kind == hm::KIND_ATTACHMENT { AttMeta::parse(&m.body) } else { None };
    ChatMessage {
        seq: m.seq,
        message_id: hex(&m.msg_id),
        contact_id: m.contact_id.clone(),
        outgoing: m.outgoing,
        text: match &att {
            Some(a) => a.label().to_string(),
            None if m.kind == hm::KIND_ATTACHMENT => UNSUPPORTED_TEXT.to_string(),
            None => m.body.clone(),
        },
        sent_at_ms: m.sent_at,
        local_at_ms: m.local_at,
        status: MessageStatus::from_db(m.status),
        attachment: att.map(|a| a.info()),
        reactions: Vec::new(),
        sender: m.sender.clone(),
    }
}

/// Junta as reacoes guardadas a estas mensagens da conversa `conv_id`.
pub(crate) fn with_reactions(c: &st::rusqlite::Connection, conv_id: &str, mut msgs: Vec<ChatMessage>) -> Result<Vec<ChatMessage>, QtError> {
    let all = hm::reactions(c, conv_id)?;
    if all.is_empty() {
        return Ok(msgs);
    }
    for m in msgs.iter_mut() {
        let id = unhex(&m.message_id).unwrap_or_default();
        m.reactions = all
            .iter()
            .filter(|(mid, _, _)| *mid == id)
            .map(|(_, who, emoji)| Reaction { emoji: emoji.clone(), mine: who.is_empty(), from_contact: who.clone() })
            .collect();
    }
    Ok(msgs)
}

// ------------------------------------------------------------ codificacao

pub(crate) fn encode_text(id: &[u8; 16], sent_at: i64, text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(HDR + text.len());
    out.push(PAYLOAD_V1);
    out.push(T_TEXT);
    out.extend_from_slice(id);
    out.extend_from_slice(&sent_at.to_be_bytes());
    out.extend_from_slice(text.as_bytes());
    out
}

pub(crate) fn encode_attachment(id: &[u8; 16], sent_at: i64, wire: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HDR + wire.len());
    out.push(PAYLOAD_V1);
    out.push(T_ATTACHMENT);
    out.extend_from_slice(id);
    out.extend_from_slice(&sent_at.to_be_bytes());
    out.extend_from_slice(wire);
    out
}

/// Mensagem de controlo (convite, reacao...): cabecalho v1 + corpo.
pub(crate) fn encode_control(kind: u8, body: &[u8]) -> Result<Vec<u8>, QtError> {
    let id = new_id()?;
    let mut out = Vec::with_capacity(HDR + body.len());
    out.push(PAYLOAD_V1);
    out.push(kind);
    out.extend_from_slice(&id);
    out.extend_from_slice(&now_ms().to_be_bytes());
    out.extend_from_slice(body);
    Ok(out)
}

/// Se ainda nao foi, poe na outbox o convite (o meu Public ID) para `contact_id`,
/// ANTES da mensagem que se vai cifrar a seguir. Chamar dentro da transacao do envio.
pub(crate) fn queue_intro_if_needed(client: &QtClient, c: &st::rusqlite::Connection, contact_id: &str) -> Result<(), QtError> {
    let key = format!("{KV_SEND_INTRO}{contact_id}");
    if st::kv_get(c, &key)?.is_none() {
        return Ok(());
    }
    let me = crate::client::public_identity(&st::load_identity(c)?).public_id;
    let blob = client.seal_with(c, contact_id, &encode_control(T_INTRO, me.as_bytes())?)?;
    hm::outbox_put(c, &new_id()?, contact_id, hm::OUTBOX_RECEIPT, &blob)?;
    st::kv_delete(c, &key)?;
    Ok(())
}

fn encode_receipt(id: &[u8; 16], sent_at: i64, status: i64, ids: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HDR + 3 + ids.len() * 16);
    out.push(PAYLOAD_V1);
    out.push(T_RECEIPT);
    out.extend_from_slice(id);
    out.extend_from_slice(&sent_at.to_be_bytes());
    out.push(status as u8);
    out.extend_from_slice(&(ids.len() as u16).to_be_bytes());
    for i in ids {
        out.extend_from_slice(i);
    }
    out
}

#[derive(Debug, PartialEq)]
enum Payload {
    Text { id: Vec<u8>, sent_at: i64, text: String },
    /// Tipo de mensagem de uma versao MAIS RECENTE da app (ficheiros, grupos...).
    /// Mostra-se um aviso em vez de bytes sem sentido.
    Unsupported { id: Vec<u8>, sent_at: i64 },
    Receipt { status: i64, ids: Vec<Vec<u8>> },
    Attachment { id: Vec<u8>, sent_at: i64, meta: AttMeta },
    /// Convite: o Public ID de quem envia.
    Intro { public_id: String },
    /// Reacao (emoji vazio = retirar) a uma mensagem desta conversa.
    Reaction { target: Vec<u8>, emoji: String },
    /// Grupos (ver `groups.rs`): o corpo comeca pelo group_id (16 bytes).
    GroupMsg { body: Vec<u8> },
    GroupInfo { body: Vec<u8> },
    GroupLeave { body: Vec<u8> },
    /// Presenca (ver `presence.rs`): o contacto abriu ou fechou a app.
    Presence { online: bool, sent_at: i64 },
    /// O contacto apagou-me e as chaves da sessao (tipo 10).
    SessionEnd,
    /// Chamada de voz (tipo 11, ver `call.rs`): o corpo comeca pela operacao.
    Call { body: Vec<u8>, sent_at: i64 },
    /// Mensagem de controlo invalida: ignora-se, sem aviso.
    Ignored,
}

/// Descodifica o plaintext. `legacy_id` serve para clientes antigos (texto simples).
fn decode(plain: &[u8], legacy_id: &[u8]) -> Payload {
    let legacy = || Payload::Text {
        id: legacy_id.to_vec(),
        sent_at: now_ms(),
        text: String::from_utf8_lossy(plain).into_owned(),
    };
    if plain.len() < HDR || plain[0] != PAYLOAD_V1 {
        return legacy();
    }
    let id = plain[2..18].to_vec();
    let sent_at = i64::from_be_bytes(plain[18..26].try_into().unwrap());
    match plain[1] {
        T_TEXT => match std::str::from_utf8(&plain[HDR..]) {
            Ok(t) => Payload::Text { id, sent_at, text: t.to_string() },
            Err(_) => legacy(),
        },
        T_RECEIPT => {
            let b = &plain[HDR..];
            if b.len() < 3 {
                return Payload::Receipt { status: 0, ids: vec![] };
            }
            let status = b[0] as i64;
            let n = u16::from_be_bytes([b[1], b[2]]) as usize;
            if !(status == hm::ST_DELIVERED || status == hm::ST_READ) || n > MAX_RECEIPT_IDS || b.len() != 3 + n * 16 {
                return Payload::Receipt { status: 0, ids: vec![] };
            }
            Payload::Receipt { status, ids: b[3..].chunks(16).map(|c| c.to_vec()).collect() }
        }
        T_ATTACHMENT => match decode_wire(&plain[HDR..]) {
            Some(meta) => Payload::Attachment { id, sent_at, meta },
            None => Payload::Unsupported { id, sent_at },
        },
        T_REACTION => {
            let b = &plain[HDR..];
            if b.len() < 16 || b.len() > 16 + MAX_EMOJI_BYTES {
                return Payload::Ignored;
            }
            match std::str::from_utf8(&b[16..]) {
                Ok(e) if !e.contains(['\n', '\r']) => Payload::Reaction { target: b[..16].to_vec(), emoji: e.to_string() },
                _ => Payload::Ignored,
            }
        }
        T_GROUP_MSG | T_GROUP_INFO | T_GROUP_LEAVE => {
            let body = plain[HDR..].to_vec();
            if body.len() < 16 || body.len() > 12_000 {
                return Payload::Ignored;
            }
            match plain[1] {
                T_GROUP_MSG => Payload::GroupMsg { body },
                T_GROUP_INFO => Payload::GroupInfo { body },
                _ => Payload::GroupLeave { body },
            }
        }
        crate::presence::T_PRESENCE => match &plain[HDR..] {
            [state @ (0 | 1)] => Payload::Presence { online: *state == 1, sent_at },
            _ => Payload::Ignored,
        },
        T_SESSION_END => Payload::SessionEnd,
        crate::call::T_CALL => match plain.len() - HDR {
            17..=128 => Payload::Call { body: plain[HDR..].to_vec(), sent_at },
            _ => Payload::Ignored,
        },
        T_INTRO => match std::str::from_utf8(&plain[HDR..]) {
            Ok(p) if p.len() <= 200 => Payload::Intro { public_id: p.to_string() },
            _ => Payload::Ignored,
        },
        _ => Payload::Unsupported { id, sent_at },
    }
}

fn legacy_msg_id(envelope_id: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(b"QT-LEGACY-MSG-ID");
    h.update(envelope_id);
    h.finalize()[..16].to_vec()
}

fn parse_msg_id(s: &str) -> Result<Vec<u8>, QtError> {
    match unhex(s) {
        Some(b) if b.len() == 16 => Ok(b),
        _ => Err(bad("message_id")),
    }
}

// --------------------------------------------------------------- recibos

fn pending_delivered(c: &st::rusqlite::Connection, contact_id: &str) -> Result<Vec<u8>, QtError> {
    Ok(st::kv_get(c, &format!("{KV_PENDING_DELIVERED}{contact_id}"))?.unwrap_or_default())
}

fn queue_receipts(
    client: &QtClient,
    c: &st::rusqlite::Connection,
    contact_id: &str,
    status: i64,
    ids: &[Vec<u8>],
) -> Result<u32, QtError> {
    let mut n = 0;
    for chunk in ids.chunks(MAX_RECEIPT_IDS) {
        let rid = new_id()?;
        let plain = encode_receipt(&rid, now_ms(), status, chunk);
        let blob = client.seal_with(c, contact_id, &plain)?;
        hm::outbox_put(c, &rid, contact_id, hm::OUTBOX_RECEIPT, &blob)?;
        n += 1;
    }
    Ok(n)
}

// --------------------------------------------------------------- bloqueio

/// Contactos bloqueados: so neste telemovel, dentro da base cifrada (`kv`).
/// O relay nao sabe de nada (com sealed sender nem sabe quem envia).
const KV_BLOCKED: &str = "blocked:";

pub(crate) fn is_blocked_in(c: &st::rusqlite::Connection, contact_id: &str) -> Result<bool, QtError> {
    Ok(st::kv_get(c, &format!("{KV_BLOCKED}{contact_id}"))?.is_some())
}

/// Erro de quem tenta cifrar algo para um contacto bloqueado.
pub(crate) fn blocked_error() -> QtError {
    bad("blocked")
}

#[uniffi::export]
impl QtClient {
    /// BLOQUEAR: as mensagens deste contacto passam a ser descartadas ao chegar
    /// (nada e guardado nem mostrado, e nao se enviam recibos de entregue/visto);
    /// deixa de ser possivel enviar-lhe mensagens e o que estava por enviar sai da
    /// outbox. O contacto nao e avisado. A conversa e a sessao mantem-se: ao
    /// desbloquear, tudo volta a funcionar (as mensagens recebidas entretanto ja
    /// nao voltam). Funciona tambem para quem nao esta nos contactos.
    pub fn block_contact(&self, contact_id: String) -> Result<(), QtError> {
        let _ = contact_to_identity(&contact_id)?;
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        st::kv_set(&tx, &format!("{KV_BLOCKED}{contact_id}"), &[1])?;
        st::kv_delete(&tx, &format!("{KV_PENDING_DELIVERED}{contact_id}"))?;
        tx.execute("DELETE FROM outbox WHERE contact_id = ?1", [&contact_id])?;
        tx.commit()?;
        Ok(())
    }

    /// DESBLOQUEAR: volta a receber e a poder enviar.
    pub fn unblock_contact(&self, contact_id: String) -> Result<(), QtError> {
        let _ = contact_to_identity(&contact_id)?;
        let c = self.lock()?;
        st::kv_delete(&c, &format!("{KV_BLOCKED}{contact_id}"))?;
        Ok(())
    }

    pub fn is_blocked(&self, contact_id: String) -> Result<bool, QtError> {
        let c = self.lock()?;
        is_blocked_in(&c, &contact_id)
    }

    /// CONVITE: o Public ID de quem me escreveu (ainda nao esta nos contactos), se
    /// mandou um. Com ele, a app adiciona o contacto sem pedir o ID ("Aceitar").
    pub fn pending_intro(&self, contact_id: String) -> Result<Option<String>, QtError> {
        let c = self.lock()?;
        Ok(st::kv_get(&c, &format!("{KV_INTRO}{contact_id}"))?.and_then(|v| String::from_utf8(v).ok()))
    }

    /// Contactos com convite recebido (contact_id).
    pub fn pending_intros(&self) -> Result<Vec<String>, QtError> {
        let c = self.lock()?;
        let mut stmt = c.prepare("SELECT k FROM kv WHERE k LIKE ?1 ORDER BY k")?;
        let rows = stmt.query_map([format!("{KV_INTRO}%")], |r| r.get::<_, String>(0))?;
        let keys = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(keys.into_iter().map(|k| k[KV_INTRO.len()..].to_string()).collect())
    }

    /// O convite foi tratado (aceite: o contacto ja esta na lista da app).
    pub fn dismiss_intro(&self, contact_id: String) -> Result<(), QtError> {
        let c = self.lock()?;
        st::kv_delete(&c, &format!("{KV_INTRO}{contact_id}"))?;
        Ok(())
    }

    /// Todos os contactos bloqueados (contact_id).
    pub fn blocked_contacts(&self) -> Result<Vec<String>, QtError> {
        let c = self.lock()?;
        let mut stmt = c.prepare("SELECT k FROM kv WHERE k LIKE ?1 ORDER BY k")?;
        let rows = stmt.query_map([format!("{KV_BLOCKED}%")], |r| r.get::<_, String>(0))?;
        let keys = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(keys.into_iter().map(|k| k[KV_BLOCKED.len()..].to_string()).collect())
    }
}

#[uniffi::export]
impl QtClient {
    /// Envia texto a um contacto (a sessao tem de existir: ver `start_session`).
    /// Numa so transacao: avanca o ratchet, guarda no historico como "por
    /// enviar" e poe o envelope na outbox. A app envia depois `item.blob` ao
    /// relay e, quando ele confirmar, chama `outbox_sent(item.outbox_id)`.
    pub fn send_text(&self, contact_id: String, text: String) -> Result<OutboxItem, QtError> {
        if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
            return Err(bad("text"));
        }
        let id = new_id()?;
        let now = now_ms();
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        queue_intro_if_needed(self, &tx, &contact_id)?;
        let blob = self.seal_with(&tx, &contact_id, &encode_text(&id, now, &text))?;
        hm::insert_message(&tx, &id, &contact_id, true, hm::KIND_TEXT, &text, now, now, hm::ST_PENDING)?;
        hm::outbox_put(&tx, &id, &contact_id, hm::OUTBOX_MESSAGE, &blob)?;
        tx.commit()?;
        Ok(OutboxItem { outbox_id: hex(&id), contact_id, blob, silent: false, relay_id: None })
    }

    /// REAGIR a uma mensagem desta conversa (minha ou do contacto) com um emoji;
    /// `emoji` vazio retira a minha reacao. Guarda e poe o aviso cifrado na outbox.
    pub fn react(&self, contact_id: String, message_id: String, emoji: String) -> Result<(), QtError> {
        let id = parse_msg_id(&message_id)?;
        if emoji.len() > MAX_EMOJI_BYTES || emoji.contains(['\n', '\r']) {
            return Err(bad("emoji"));
        }
        let c = self.lock()?;
        if hm::get_message(&c, &contact_id, &id)?.is_none() {
            return Err(bad("message_id"));
        }
        let tx = c.unchecked_transaction()?;
        hm::set_reaction(&tx, &contact_id, &id, "", &emoji)?;
        let mut body = id.clone();
        body.extend_from_slice(emoji.as_bytes());
        let blob = self.seal_with(&tx, &contact_id, &encode_control(T_REACTION, &body)?)?;
        hm::outbox_put(&tx, &new_id()?, &contact_id, hm::OUTBOX_RECEIPT, &blob)?;
        tx.commit()?;
        Ok(())
    }

    /// Tudo o que ainda nao foi confirmado pelo relay (reenviar apos falhas
    /// de rede ou reinicio da app), pela ordem em que foi criado.
    pub fn outbox(&self) -> Result<Vec<OutboxItem>, QtError> {
        let c = self.lock()?;
        // Contactos que terminaram a sessao: o que la esta foi cifrado com a sessao antiga
        // e volta a ser cifrado quando abrir a nova (`requeue_after_session_reset`).
        let restart: std::collections::HashSet<String> = {
            let mut stmt = c.prepare("SELECT k FROM kv WHERE k LIKE ?1")?;
            let rows = stmt.query_map([format!("{KV_RESTART}%")], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?.into_iter().map(|k| k[KV_RESTART.len()..].to_string()).collect()
        };
        hm::outbox_all(&c)?
            .into_iter()
            .filter(|e| !restart.contains(&e.contact_id))
            .map(|e| -> Result<OutboxItem, QtError> {
                let silent = match e.kind {
                    hm::OUTBOX_RECEIPT => true,
                    hm::OUTBOX_GROUP => crate::groups::is_silent_envelope(&c, &e.outbox_id)?,
                    _ => false,
                };
                let relay_id = st::kv_get(&c, &format!("{KV_OUT_RELAY}{}", hex(&e.outbox_id)))?.and_then(|v| String::from_utf8(v).ok());
                Ok(OutboxItem { outbox_id: hex(&e.outbox_id), contact_id: e.contact_id, silent, relay_id, blob: e.blob })
            })
            .collect()
    }

    /// O relay confirmou (`Ack`) o envio deste item: sai da outbox e, se era
    /// uma mensagem, passa a "Enviado".
    pub fn outbox_sent(&self, outbox_id: String) -> Result<(), QtError> {
        let id = parse_msg_id(&outbox_id)?;
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        st::kv_delete(&tx, &format!("{KV_OUT_RELAY}{}", hex(&id)))?;
        if let Some(e) = hm::outbox_take(&tx, &id)? {
            if e.kind == hm::OUTBOX_MESSAGE {
                hm::mark_sent(&tx, &id)?;
            } else if e.kind == hm::OUTBOX_GROUP {
                crate::groups::on_outbox_sent(&tx, &id)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Recebe um envelope selado vindo do relay: decifra, guarda e agenda o
    /// recibo "entregue", tudo numa transacao. So confirmar (`Ack`) ao relay
    /// DEPOIS disto devolver Ok (ou um erro definitivo como `Replay`).
    pub fn receive_sealed(&self, sealed_envelope: Vec<u8>) -> Result<ReceiveResult, QtError> {
        let c = self.lock()?;
        let mut result: Option<ReceiveResult> = None;
        let mut hook = |tx: &st::rusqlite::Connection, contact_id: &str, env_id: &[u8], plain: &[u8]| -> Result<(), QtError> {
            // Bloqueado: a sessao avanca (para continuar a funcionar se for desbloqueado), mas
            // nada e guardado nem mostrado e nao se agenda recibo. Quem enviou nao e avisado.
            if is_blocked_in(tx, contact_id)? {
                result = Some(ReceiveResult::NoChange { contact_id: contact_id.to_string() });
                return Ok(());
            }
            let now = now_ms();
            result = Some(match decode(plain, &legacy_msg_id(env_id)) {
                Payload::Text { id, sent_at, text } => {
                    match hm::insert_message(tx, &id, contact_id, false, hm::KIND_TEXT, &text, sent_at, now, hm::ST_RECEIVED)? {
                        Some(_) => {
                            // Recibo "entregue" agendado (enviado em lote por `prepare_delivery_receipts`).
                            let mut p = pending_delivered(tx, contact_id)?;
                            if p.len() < MAX_RECEIPT_IDS * 16 * 4 {
                                p.extend_from_slice(&id);
                                st::kv_set(tx, &format!("{KV_PENDING_DELIVERED}{contact_id}"), &p)?;
                            }
                            let m = hm::get_message(tx, contact_id, &id)?.ok_or(QtError::Storage)?;
                            ReceiveResult::NewMessage { message: to_chat(&m) }
                        }
                        None => ReceiveResult::NoChange { contact_id: contact_id.to_string() },
                    }
                }
                Payload::Receipt { status, ids } if !ids.is_empty() => {
                    let changed = hm::raise_outgoing_status(tx, contact_id, &ids, status)?;
                    if changed == 0 {
                        ReceiveResult::NoChange { contact_id: contact_id.to_string() }
                    } else {
                        ReceiveResult::StatusChanged {
                            contact_id: contact_id.to_string(),
                            message_ids: ids.iter().map(|i| hex(i)).collect(),
                            status: MessageStatus::from_db(status),
                        }
                    }
                }
                Payload::Receipt { .. } => ReceiveResult::NoChange { contact_id: contact_id.to_string() },
                Payload::Attachment { id, sent_at, meta } => {
                    let body = received_body(&meta)?;
                    match hm::insert_message(tx, &id, contact_id, false, hm::KIND_ATTACHMENT, &body, sent_at, now, hm::ST_RECEIVED)? {
                        Some(_) => {
                            let mut p = pending_delivered(tx, contact_id)?;
                            if p.len() < MAX_RECEIPT_IDS * 16 * 4 {
                                p.extend_from_slice(&id);
                                st::kv_set(tx, &format!("{KV_PENDING_DELIVERED}{contact_id}"), &p)?;
                            }
                            let m = hm::get_message(tx, contact_id, &id)?.ok_or(QtError::Storage)?;
                            ReceiveResult::NewMessage { message: to_chat(&m) }
                        }
                        None => ReceiveResult::NoChange { contact_id: contact_id.to_string() },
                    }
                }
                Payload::Intro { public_id } => {
                    // So vale se o ID for mesmo de quem enviou (a identidade X25519 tem de ser
                    // a desta sessao). A parte de assinatura so serve para lhe responder.
                    match crate::client::parse_public_id(&public_id) {
                        Ok((id_pub, _)) if hex(&id_pub) == contact_id => {
                            st::kv_set(tx, &format!("{KV_INTRO}{contact_id}"), public_id.as_bytes())?;
                        }
                        _ => {}
                    }
                    ReceiveResult::NoChange { contact_id: contact_id.to_string() }
                }
                Payload::Reaction { target, emoji } => {
                    // So a mensagens desta conversa (minhas ou dele).
                    if hm::get_message(tx, contact_id, &target)?.is_some() {
                        hm::set_reaction(tx, contact_id, &target, contact_id, &emoji)?;
                        ReceiveResult::Updated { contact_id: contact_id.to_string() }
                    } else {
                        ReceiveResult::NoChange { contact_id: contact_id.to_string() }
                    }
                }
                Payload::GroupMsg { body } => crate::groups::on_group_msg(tx, contact_id, &body)?,
                Payload::GroupInfo { body } => crate::groups::on_group_info(tx, contact_id, &body)?,
                Payload::GroupLeave { body } => crate::groups::on_group_leave(self, tx, contact_id, &body)?,
                Payload::Presence { online, sent_at } => crate::presence::on_presence(self, tx, contact_id, online, sent_at)?,
                Payload::SessionEnd => on_session_end(tx, contact_id)?,
                Payload::Call { body, sent_at } => crate::call::on_call(self, tx, contact_id, &body, sent_at)?,
                Payload::Ignored => ReceiveResult::NoChange { contact_id: contact_id.to_string() },
                Payload::Unsupported { id, sent_at } => {
                    match hm::insert_message(tx, &id, contact_id, false, hm::KIND_TEXT, UNSUPPORTED_TEXT, sent_at, now, hm::ST_RECEIVED)? {
                        Some(_) => {
                            let m = hm::get_message(tx, contact_id, &id)?.ok_or(QtError::Storage)?;
                            ReceiveResult::NewMessage { message: to_chat(&m) }
                        }
                        None => ReceiveResult::NoChange { contact_id: contact_id.to_string() },
                    }
                }
            });
            Ok(())
        };
        self.open_sealed_with(&c, &sealed_envelope, &mut hook)?;
        result.ok_or(QtError::Storage)
    }

    /// Junta os recibos "entregue" pendentes (um envelope por contacto) e
    /// poe-nos na outbox. Chamar depois de processar um lote de mensagens.
    /// Devolve quantos envelopes foram criados.
    pub fn prepare_delivery_receipts(&self) -> Result<u32, QtError> {
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        let contacts: Vec<String> = {
            let mut stmt = tx.prepare("SELECT k FROM kv WHERE k LIKE ?1")?;
            let rows = stmt.query_map([format!("{KV_PENDING_DELIVERED}%")], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut n = 0;
        for key in contacts {
            let contact_id = key[KV_PENDING_DELIVERED.len()..].to_string();
            let raw = st::kv_get(&tx, &key)?.unwrap_or_default();
            let ids: Vec<Vec<u8>> = raw.chunks(16).filter(|c| c.len() == 16).map(|c| c.to_vec()).collect();
            if !ids.is_empty() && !is_blocked_in(&tx, &contact_id)? && st::load_session(&tx, &contact_id)?.is_some() {
                n += queue_receipts(self, &tx, &contact_id, hm::ST_DELIVERED, &ids)?;
            }
            st::kv_delete(&tx, &key)?;
        }
        tx.commit()?;
        Ok(n)
    }

    /// O utilizador abriu a conversa: marca as recebidas como lidas e, se
    /// `send_read_receipt`, poe na outbox o recibo "vista". Devolve quantas
    /// mensagens passaram a lidas.
    pub fn mark_conversation_read(&self, contact_id: String, send_read_receipt: bool) -> Result<u32, QtError> {
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        let ids = hm::mark_incoming_read(&tx, &contact_id)?;
        if send_read_receipt && !ids.is_empty() && !is_blocked_in(&tx, &contact_id)? && st::load_session(&tx, &contact_id)?.is_some() {
            queue_receipts(self, &tx, &contact_id, hm::ST_READ, &ids)?;
        }
        tx.commit()?;
        Ok(ids.len() as u32)
    }

    /// Pagina do historico, da mais RECENTE para a mais antiga. Primeira pagina:
    /// `before_seq = None`; seguintes: o menor `seq` ja carregado.
    pub fn messages_page(&self, contact_id: String, before_seq: Option<i64>, limit: u32) -> Result<Vec<ChatMessage>, QtError> {
        let c = self.lock()?;
        let msgs = hm::messages_page(&c, &contact_id, before_seq, limit)?.iter().map(to_chat).collect();
        with_reactions(&c, &contact_id, msgs)
    }

    /// Mensagens mais recentes que `after_seq` (da mais antiga para a mais recente).
    pub fn messages_after(&self, contact_id: String, after_seq: i64) -> Result<Vec<ChatMessage>, QtError> {
        let c = self.lock()?;
        let msgs = hm::messages_after(&c, &contact_id, after_seq)?.iter().map(to_chat).collect();
        with_reactions(&c, &contact_id, msgs)
    }

    /// Estado atual de uma mensagem (para atualizar os visto/entregue no ecra).
    pub fn message(&self, contact_id: String, message_id: String) -> Result<Option<ChatMessage>, QtError> {
        let id = parse_msg_id(&message_id)?;
        let c = self.lock()?;
        let m: Vec<ChatMessage> = hm::get_message(&c, &contact_id, &id)?.as_ref().map(to_chat).into_iter().collect();
        Ok(with_reactions(&c, &contact_id, m)?.pop())
    }

    /// Lista de conversas (ultima mensagem + por ler), a mais recente primeiro.
    pub fn conversations(&self) -> Result<Vec<ConversationSummary>, QtError> {
        let c = self.lock()?;
        Ok(hm::conversations(&c)?
            .iter()
            .map(|s| ConversationSummary { contact_id: s.contact_id.clone(), last_message: to_chat(&s.last), unread: s.unread as u32 })
            .collect())
    }

    /// ELIMINAR CONVERSA: apaga de forma permanente todas as mensagens desta
    /// conversa (e o que ainda estava por enviar). A sessao cifrada mantem-se,
    /// por isso a conversa pode continuar. Com `secure_delete` as paginas
    /// apagadas sao sobrescritas no ficheiro (nao ficam recuperaveis).
    pub fn delete_conversation(&self, contact_id: String) -> Result<(), QtError> {
        let _ = contact_to_identity(&contact_id)?;
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        self.remove_contact_files(&tx, &contact_id)?;
        hm::delete_conversation(&tx, &contact_id)?;
        st::kv_delete(&tx, &format!("{KV_PENDING_DELIVERED}{contact_id}"))?;
        tx.commit()?;
        Ok(())
    }

    /// APAGAR CONTACTO: apaga de forma permanente as mensagens, o que estava
    /// por enviar, a sessao cifrada (chaves do Double Ratchet) e os metadados
    /// deste contacto. Os registos anti-replay passam a hashes anonimos (sem o
    /// contacto em claro), para o relay nao conseguir reentregar mensagens
    /// antigas. Se o contacto voltar a ser adicionado, comeca uma sessao nova.
    pub fn delete_contact(&self, contact_id: String) -> Result<(), QtError> {
        self.delete_contact_inner(contact_id, None)
    }

    /// Como `delete_contact`, mas antes de apagar as chaves poe na outbox o aviso
    /// "sessao terminada" (tipo 10, cifrado com a sessao que vai ser apagada) para
    /// `relay_id` (o endereco do contacto no relay). Sem isto, o contacto continuava a
    /// cifrar com a sessao antiga e as mensagens dele perdiam-se. Ao receber o aviso,
    /// a app dele apaga a sessao e a proxima mensagem abre uma nova (aqui aparece como
    /// convite). Bloqueado ou sem sessao: nao avisa (apaga na mesma).
    pub fn delete_contact_notify(&self, contact_id: String, relay_id: String) -> Result<(), QtError> {
        let ok = relay_id.len() == 64 && relay_id.bytes().all(|b| b.is_ascii_hexdigit());
        if !ok {
            return Err(bad("relay_id"));
        }
        self.delete_contact_inner(contact_id, Some(relay_id))
    }
}

/// Tipo 10 recebido (ja autenticado pelo AEAD da sessao, dentro da transacao da
/// rececao, DEPOIS de a sessao avancada ter sido guardada): o contacto apagou as
/// chaves dele. Apaga a nossa sessao (o contacto e o historico ficam) e marca que
/// falta abrir uma nova; a outbox dele fica retida ate la (ver `outbox`).
fn on_session_end(tx: &st::rusqlite::Connection, contact_id: &str) -> Result<ReceiveResult, QtError> {
    st::delete_session(tx, contact_id)?;
    st::kv_delete(tx, &format!("pending:{contact_id}"))?;
    st::kv_delete(tx, &format!("{KV_PENDING_DELIVERED}{contact_id}"))?;
    st::kv_set(tx, &format!("{KV_RESTART}{contact_id}"), &[1])?;
    Ok(ReceiveResult::NoChange { contact_id: contact_id.to_string() })
}

/// Sessao nova instalada para `contact_id` (`start_session` ou recebida). Se ele
/// tinha terminado a anterior, volta a cifrar o que estava por entregar (mensagens,
/// envelopes de grupo) com a nova, depois do convite. Corre na transacao da sessao.
pub(crate) fn after_new_session(client: &QtClient, tx: &st::rusqlite::Connection, contact_id: &str) -> Result<(), QtError> {
    let key = format!("{KV_RESTART}{contact_id}");
    if st::kv_get(tx, &key)?.is_none() || is_blocked_in(tx, contact_id)? {
        return Ok(());
    }
    st::kv_delete(tx, &key)?;
    // Primeiro o que estava por entregar (isto limpa a outbox antiga dele), depois o convite.
    requeue_after_session_reset(client, tx, contact_id)?;
    queue_intro_if_needed(client, tx, contact_id)
}

impl QtClient {
    pub(crate) fn delete_contact_inner(&self, contact_id: String, notify_relay: Option<String>) -> Result<(), QtError> {
        let _ = contact_to_identity(&contact_id)?;
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        self.remove_contact_files(&tx, &contact_id)?;
        hm::delete_conversation(&tx, &contact_id)?;
        // Depois de limpar a outbox dele (fica so o aviso) e antes de apagar a sessao.
        if let Some(relay) = notify_relay {
            if !is_blocked_in(&tx, &contact_id)? && st::load_session(&tx, &contact_id)?.is_some() {
                let blob = self.seal_with(&tx, &contact_id, &encode_control(T_SESSION_END, &[])?)?;
                let out = new_id()?;
                hm::outbox_put(&tx, &out, &contact_id, hm::OUTBOX_RECEIPT, &blob)?;
                st::kv_set(&tx, &format!("{KV_OUT_RELAY}{}", hex(&out)), relay.as_bytes())?;
            }
        }
        st::kv_delete(&tx, &format!("{KV_RESTART}{contact_id}"))?;
        st::kv_delete(&tx, &format!("{KV_PENDING_DELIVERED}{contact_id}"))?;
        st::kv_delete(&tx, &format!("pending:{contact_id}"))?;
        st::kv_delete(&tx, &format!("{}{contact_id}", crate::client::KV_PEER_EPH))?;
        st::kv_delete(&tx, &format!("{KV_INTRO}{contact_id}"))?;
        st::kv_delete(&tx, &format!("{KV_SEND_INTRO}{contact_id}"))?;
        st::delete_session(&tx, &contact_id)?;
        tx.commit()?;
        Ok(())
    }
}

#[uniffi::export]
impl QtClient {
    /// Contactos que terminaram a sessao (tipo 10) e com quem ainda nao ha sessao
    /// nova. A app abre uma (`relay_request_fetch_bundle` + `start_session`) para o
    /// que estava por entregar seguir logo, sem esperar pela proxima mensagem.
    pub fn sessions_to_restart(&self) -> Result<Vec<String>, QtError> {
        let c = self.lock()?;
        let mut stmt = c.prepare("SELECT k FROM kv WHERE k LIKE ?1")?;
        let rows = stmt.query_map([format!("{KV_RESTART}%")], |r| r.get::<_, String>(0))?;
        let keys = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(keys.into_iter().map(|k| k[KV_RESTART.len()..].to_string()).collect())
    }
}

/// Texto mostrado quando chega um tipo de mensagem que esta versao nao conhece.
pub const UNSUPPORTED_TEXT: &str =
    "⚠️ Esta mensagem usa uma funcionalidade que esta versão da app ainda não suporta. Atualiza o SEALER para a ver.";

/// A sessao com `contact_id` foi substituida por uma nova (sessoes cruzadas ou
/// o contacto reiniciou a dele): os envelopes da outbox e as mensagens ainda
/// sem recibo "entregue" foram cifrados com a sessao antiga, que o contacto ja
/// nao consegue ler. Volta a cifra-las com a sessao nova (mesmo id, por isso o
/// destinatario nunca as mostra em duplicado) e volta a pô-las na outbox.
/// Corre DENTRO da transacao que instala a sessao nova.
pub(crate) fn requeue_after_session_reset(client: &QtClient, tx: &st::rusqlite::Connection, contact_id: &str) -> Result<(), QtError> {
    crate::groups::requeue_outbox_of(client, tx, contact_id)?;
    hm::outbox_delete_contact(tx, contact_id)?;
    for m in hm::outgoing_undelivered(tx, contact_id, 200)? {
        let Ok(id) = <[u8; 16]>::try_from(m.msg_id.as_slice()) else { continue };
        let plain = match m.kind {
            hm::KIND_TEXT => encode_text(&id, m.sent_at, &m.body),
            // Anexos ja enviados ao relay: a mensagem com a chave volta a seguir. Os
            // que ainda estao a ser enviados saem quando o envio acabar (ja com a sessao nova).
            hm::KIND_ATTACHMENT => match AttMeta::parse(&m.body) {
                Some(a) if a.blob.is_some() && a.token.is_none() && a.info().state == crate::attachments::AttachmentState::Ready => {
                    encode_attachment(&id, m.sent_at, &encode_wire(&a)?)
                }
                _ => continue,
            },
            _ => continue,
        };
        let blob = client.seal_with(tx, contact_id, &plain)?;
        hm::outbox_put(tx, &id, contact_id, hm::OUTBOX_MESSAGE, &blob)?;
        hm::reset_outgoing_to_pending(tx, contact_id, &id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tipo_desconhecido_de_versao_nova_vira_aviso_e_nao_lixo() {
        let id = [7u8; 16];
        let mut p = encode_text(&id, 123, "x");
        p[1] = 99; // tipo que esta versao nao conhece
        assert_eq!(decode(&p, &[0; 16]), Payload::Unsupported { id: id.to_vec(), sent_at: 123 });
    }

    #[test]
    fn texto_e_recibo_ida_e_volta() {
        let id = [7u8; 16];
        let p = encode_text(&id, 1234, "olá 👋");
        assert_eq!(decode(&p, &[0; 16]), Payload::Text { id: id.to_vec(), sent_at: 1234, text: "olá 👋".into() });
        let ids = vec![vec![1u8; 16], vec![2u8; 16]];
        let r = encode_receipt(&id, 9, hm::ST_READ, &ids);
        assert_eq!(decode(&r, &[0; 16]), Payload::Receipt { status: hm::ST_READ, ids });
    }

    #[test]
    fn texto_de_cliente_antigo_e_recibos_malformados() {
        match decode(b"mensagem antiga", &[5u8; 16]) {
            Payload::Text { id, text, .. } => {
                assert_eq!(id, vec![5u8; 16]);
                assert_eq!(text, "mensagem antiga");
            }
            other => panic!("{other:?}"),
        }
        // Recibo com estado invalido ou tamanho errado: ignorado (sem ids).
        let mut r = encode_receipt(&[1; 16], 0, hm::ST_READ, &[vec![3u8; 16]]);
        r[HDR] = 99;
        assert_eq!(decode(&r, &[0; 16]), Payload::Receipt { status: 0, ids: vec![] });
        let mut r = encode_receipt(&[1; 16], 0, hm::ST_READ, &[vec![3u8; 16]]);
        r.pop();
        assert_eq!(decode(&r, &[0; 16]), Payload::Receipt { status: 0, ids: vec![] });
    }
}
