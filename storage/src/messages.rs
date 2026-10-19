//! Historico de mensagens e fila de saida (outbox), dentro do SQLCipher.
//!
//! Antes, por desenho, este crate NAO guardava texto de mensagens. Isso
//! deixava a app sem historico e criava uma janela de PERDA: o ratchet
//! avancava e o envelope era marcado como visto, mas o texto so existia em
//! memoria. Agora o texto e guardado NA MESMA TRANSACAO que o avanco do
//! ratchet (ver qt_ffi::client), sempre dentro da base cifrada, com
//! `secure_delete` ativo (o que se apaga e sobrescrito).
//!
//! Estados (`status`):
//!   saida:   0 = por enviar (cifrada, na outbox) | 1 = enviada (relay confirmou)
//!            2 = entregue (recibo cifrado do destinatario) | 3 = vista
//!   entrada: 10 = recebida, por ler | 11 = lida por mim

use crate::StorageError;
use rusqlite::{params, Connection, OptionalExtension};

pub const ST_PENDING: i64 = 0;
pub const ST_SENT: i64 = 1;
pub const ST_DELIVERED: i64 = 2;
pub const ST_READ: i64 = 3;
pub const ST_RECEIVED: i64 = 10; // usado tambem no indice parcial `messages_unread`
pub const ST_READ_BY_ME: i64 = 11;

pub const KIND_TEXT: i64 = 1;
/// Imagem ou GIF. `body` guarda os metadados em JSON (chave do ficheiro,
/// tamanho, tipo, estado do envio/descarga); os bytes ficam num ficheiro cifrado.
pub const KIND_ATTACHMENT: i64 = 2;

/// Tipo de entrada na outbox (o que a app tem de fazer quando o relay confirma).
pub const OUTBOX_MESSAGE: i64 = 1;
pub const OUTBOX_RECEIPT: i64 = 2;
/// Envelope de uma mensagem de grupo para um membro (a mensagem fica "Enviada"
/// quando o relay confirmar os de todos os membros; ver qt_ffi::groups).
pub const OUTBOX_GROUP: i64 = 3;
/// Oferta de chamada de voz: toca a campainha marcada como chamada (uma app com o
/// bloqueio ativo nao a consegue decifrar, mas sabe que deve tocar).
pub const OUTBOX_CALL: i64 = 4;
/// Chamada cancelada antes de atender: a campainha marcada faz parar o toque.
pub const OUTBOX_CALL_END: i64 = 5;

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS messages (
        seq        INTEGER PRIMARY KEY AUTOINCREMENT,
        msg_id     BLOB    NOT NULL,
        contact_id TEXT    NOT NULL,
        outgoing   INTEGER NOT NULL,
        kind       INTEGER NOT NULL,
        body       TEXT    NOT NULL,
        sent_at    INTEGER NOT NULL,
        local_at   INTEGER NOT NULL,
        status     INTEGER NOT NULL,
        UNIQUE (contact_id, msg_id)
    );
    CREATE INDEX IF NOT EXISTS messages_by_contact ON messages (contact_id, seq);
    -- Sem estes dois indices, confirmar um envio e marcar uma conversa como
    -- lida percorriam o historico inteiro (o tempo crescia com cada mensagem).
    CREATE INDEX IF NOT EXISTS messages_by_msg_id ON messages (msg_id);
    CREATE INDEX IF NOT EXISTS messages_unread ON messages (contact_id) WHERE outgoing = 0 AND status = 10;
    CREATE TABLE IF NOT EXISTS outbox (
        seq        INTEGER PRIMARY KEY AUTOINCREMENT,
        outbox_id  BLOB    NOT NULL UNIQUE,
        contact_id TEXT    NOT NULL,
        kind       INTEGER NOT NULL,
        blob       BLOB    NOT NULL
    );
    -- Reacoes com emoji: uma por pessoa e por mensagem. reactor = '' sou eu;
    -- senao, o contact_id de quem reagiu. conv_id = a conversa (contact_id ou grupo).
    CREATE TABLE IF NOT EXISTS reactions (
        conv_id TEXT NOT NULL,
        msg_id  BLOB NOT NULL,
        reactor TEXT NOT NULL,
        emoji   TEXT NOT NULL,
        PRIMARY KEY (conv_id, msg_id, reactor)
    );
    -- Grupos: info em JSON (nome, versao, membros com Public ID e se sao admins).
    -- active = 0: sai do grupo ou fui removido (o historico fica ate o apagar).
    CREATE TABLE IF NOT EXISTS groups (
        group_id TEXT PRIMARY KEY,
        info     TEXT NOT NULL,
        active   INTEGER NOT NULL DEFAULT 1
    );";

/// Atualiza bases de dados criadas por versoes anteriores (colunas novas).
pub(crate) fn migrate(conn: &Connection) -> Result<(), StorageError> {
    // `sender`: quem escreveu a mensagem num grupo ('' nas conversas 1 a 1 e nas minhas).
    let has_sender = conn.prepare("SELECT 1 FROM pragma_table_info('messages') WHERE name = 'sender'")?.exists([])?;
    if !has_sender {
        conn.execute_batch("ALTER TABLE messages ADD COLUMN sender TEXT NOT NULL DEFAULT ''")?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMessage {
    pub seq: i64,
    pub msg_id: Vec<u8>,
    pub contact_id: String,
    pub outgoing: bool,
    pub kind: i64,
    pub body: String,
    /// Relogio de quem ENVIOU (ms desde 1970). Informativo: pode estar errado.
    pub sent_at: i64,
    /// Relogio deste aparelho quando a mensagem foi criada/recebida (ms).
    pub local_at: i64,
    pub status: i64,
    /// Num grupo: o contact_id de quem escreveu. Vazio nas conversas 1 a 1 e nas minhas.
    pub sender: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationSummary {
    pub contact_id: String,
    pub last: StoredMessage,
    pub unread: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEntry {
    pub outbox_id: Vec<u8>,
    pub contact_id: String,
    pub kind: i64,
    pub blob: Vec<u8>,
}

fn row_to_msg(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMessage> {
    Ok(StoredMessage {
        seq: r.get(0)?,
        msg_id: r.get(1)?,
        contact_id: r.get(2)?,
        outgoing: r.get::<_, i64>(3)? != 0,
        kind: r.get(4)?,
        body: r.get(5)?,
        sent_at: r.get(6)?,
        local_at: r.get(7)?,
        status: r.get(8)?,
        sender: r.get(9)?,
    })
}

const COLS: &str = "seq, msg_id, contact_id, outgoing, kind, body, sent_at, local_at, status, sender";

/// Insere uma mensagem. Devolve `Some(seq)` se for nova e `None` se ja existia
/// (mesmo `msg_id` do mesmo contacto: entrega duplicada, ignorada).
#[allow(clippy::too_many_arguments)] // um argumento por coluna da mensagem
pub fn insert_message(
    conn: &Connection,
    msg_id: &[u8],
    contact_id: &str,
    outgoing: bool,
    kind: i64,
    body: &str,
    sent_at: i64,
    local_at: i64,
    status: i64,
) -> Result<Option<i64>, StorageError> {
    insert_message_from(conn, msg_id, contact_id, outgoing, kind, body, sent_at, local_at, status, "")
}

/// Como `insert_message`, com quem escreveu (`sender`, nas mensagens de grupo).
#[allow(clippy::too_many_arguments)] // um argumento por coluna da mensagem
pub fn insert_message_from(
    conn: &Connection,
    msg_id: &[u8],
    contact_id: &str,
    outgoing: bool,
    kind: i64,
    body: &str,
    sent_at: i64,
    local_at: i64,
    status: i64,
    sender: &str,
) -> Result<Option<i64>, StorageError> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO messages (msg_id, contact_id, outgoing, kind, body, sent_at, local_at, status, sender)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![msg_id, contact_id, outgoing as i64, kind, body, sent_at, local_at, status, sender],
    )?;
    Ok(if n == 1 { Some(conn.last_insert_rowid()) } else { None })
}

/// Sobe o estado de mensagens ENVIADAS a `contact_id` (nunca desce: um recibo
/// "entregue" atrasado nao apaga um "visto"). So afeta mensagens desse
/// contacto: um contacto nao consegue mexer em mensagens de outro.
pub fn raise_outgoing_status(
    conn: &Connection,
    contact_id: &str,
    msg_ids: &[Vec<u8>],
    new_status: i64,
) -> Result<usize, StorageError> {
    let mut total = 0;
    let mut stmt = conn.prepare(
        "UPDATE messages SET status = ?1
         WHERE contact_id = ?2 AND msg_id = ?3 AND outgoing = 1 AND status < ?1",
    )?;
    for id in msg_ids {
        total += stmt.execute(params![new_status, contact_id, id])?;
    }
    Ok(total)
}

/// Marca como "enviada" (o relay confirmou) uma mensagem ainda por enviar.
pub fn mark_sent(conn: &Connection, msg_id: &[u8]) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE messages SET status = ?1 WHERE msg_id = ?2 AND outgoing = 1 AND status = ?3",
        params![ST_SENT, msg_id, ST_PENDING],
    )?;
    Ok(())
}

/// Pagina do historico de um contacto, da mais RECENTE para a mais antiga.
/// `before_seq` = None para a primeira pagina; depois, o menor `seq` recebido.
pub fn messages_page(
    conn: &Connection,
    contact_id: &str,
    before_seq: Option<i64>,
    limit: u32,
) -> Result<Vec<StoredMessage>, StorageError> {
    let limit = limit.clamp(1, 500) as i64;
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM messages WHERE contact_id = ?1 AND seq < ?2 ORDER BY seq DESC LIMIT ?3"
    ))?;
    let rows = stmt.query_map(params![contact_id, before_seq.unwrap_or(i64::MAX), limit], row_to_msg)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Mensagens de um contacto mais RECENTES que `after_seq` (para atualizar o ecra
/// aberto sem recarregar tudo), da mais antiga para a mais recente.
pub fn messages_after(conn: &Connection, contact_id: &str, after_seq: i64) -> Result<Vec<StoredMessage>, StorageError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM messages WHERE contact_id = ?1 AND seq > ?2 ORDER BY seq ASC LIMIT 500"
    ))?;
    let rows = stmt.query_map(params![contact_id, after_seq], row_to_msg)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn get_message(conn: &Connection, contact_id: &str, msg_id: &[u8]) -> Result<Option<StoredMessage>, StorageError> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM messages WHERE contact_id = ?1 AND msg_id = ?2"),
            params![contact_id, msg_id],
            row_to_msg,
        )
        .optional()?)
}

/// Uma linha por contacto: ultima mensagem + numero de mensagens por ler,
/// ordenado pela conversa com atividade mais recente.
pub fn conversations(conn: &Connection) -> Result<Vec<ConversationSummary>, StorageError> {
    // A ultima mensagem de cada contacto vem do indice (contact_id, seq) e a
    // contagem por ler do indice parcial `messages_unread`. Desde a 0.9.12 os
    // contactos sao percorridos aos saltos no indice (`k`: o contact_id seguinte
    // a cada passo), em vez de `GROUP BY contact_id`, que lia o indice inteiro
    // (uma entrada por mensagem): o tempo deixa de crescer com o historico
    // (medido: 300 000 mensagens, de ~33 ms para ~0,2 ms, mesmo resultado).
    let mut stmt = conn.prepare(&format!(
        "WITH RECURSIVE k(cid) AS (
             SELECT MIN(contact_id) FROM messages
             UNION ALL
             SELECT (SELECT MIN(contact_id) FROM messages WHERE contact_id > k.cid) FROM k WHERE k.cid IS NOT NULL
         )
         SELECT {COLS},
                (SELECT COUNT(*) FROM messages u
                  WHERE u.contact_id = m.contact_id AND u.outgoing = 0 AND u.status = {ST_RECEIVED})
         FROM k JOIN messages m ON m.seq = (SELECT MAX(seq) FROM messages WHERE contact_id = k.cid)
         WHERE k.cid IS NOT NULL
         ORDER BY m.seq DESC"
    ))?;
    let rows = stmt.query_map([], |r| {
        let last = row_to_msg(r)?;
        Ok(ConversationSummary { contact_id: last.contact_id.clone(), last, unread: r.get(10)? })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Marca como lidas as mensagens recebidas de `contact_id` e devolve os seus
/// `msg_id` (para o recibo "vista").
pub fn mark_incoming_read(conn: &Connection, contact_id: &str) -> Result<Vec<Vec<u8>>, StorageError> {
    // O estado vai como constante no SQL (e nao como parametro) para o SQLite
    // poder usar o indice parcial `messages_unread`.
    let mut stmt = conn.prepare(&format!(
        "SELECT msg_id FROM messages WHERE contact_id = ?1 AND outgoing = 0 AND status = {ST_RECEIVED} ORDER BY seq"
    ))?;
    let ids = stmt
        .query_map(params![contact_id], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    conn.execute(
        &format!("UPDATE messages SET status = ?1 WHERE contact_id = ?2 AND outgoing = 0 AND status = {ST_RECEIVED}"),
        params![ST_READ_BY_ME, contact_id],
    )?;
    Ok(ids)
}

/// Muda o `body` de uma mensagem (metadados de um anexo). Devolve se mudou.
pub fn update_body(conn: &Connection, contact_id: &str, msg_id: &[u8], body: &str) -> Result<bool, StorageError> {
    Ok(conn.execute(
        "UPDATE messages SET body = ?1 WHERE contact_id = ?2 AND msg_id = ?3",
        params![body, contact_id, msg_id],
    )? == 1)
}

/// Anexos cujo `body` contem `body_fragment` (ex.: o estado `"st":0`), enviados
/// (`outgoing`) ou recebidos. Devolve (contacto, msg_id), dos mais antigos para
/// os mais recentes.
pub fn attachments_matching(
    conn: &Connection,
    outgoing: bool,
    body_fragment: &str,
) -> Result<Vec<(String, Vec<u8>)>, StorageError> {
    let pattern = format!("%{}%", body_fragment.replace(['%', '_'], ""));
    let mut stmt = conn.prepare(
        "SELECT contact_id, msg_id FROM messages WHERE kind = ?1 AND outgoing = ?2 AND body LIKE ?3 ORDER BY seq LIMIT 200",
    )?;
    let rows = stmt.query_map(params![KIND_ATTACHMENT, outgoing as i64, pattern], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// `msg_id` de todos os anexos de um contacto (para apagar os ficheiros).
pub fn attachment_ids(conn: &Connection, contact_id: &str) -> Result<Vec<Vec<u8>>, StorageError> {
    let mut stmt = conn.prepare("SELECT msg_id FROM messages WHERE contact_id = ?1 AND kind = ?2")?;
    let rows = stmt.query_map(params![contact_id, KIND_ATTACHMENT], |r| r.get(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Apaga o historico (e a outbox) de um contacto. Os envelopes de grupos para
/// esse contacto ficam: sao de outra conversa (a do grupo).
pub fn delete_conversation(conn: &Connection, contact_id: &str) -> Result<(), StorageError> {
    conn.execute("DELETE FROM messages WHERE contact_id = ?1", [contact_id])?;
    conn.execute("DELETE FROM outbox WHERE contact_id = ?1 AND kind != ?2", params![contact_id, OUTBOX_GROUP])?;
    conn.execute("DELETE FROM reactions WHERE conv_id = ?1", [contact_id])?;
    Ok(())
}

/// Define (ou, com `emoji` vazio, retira) a reacao de `reactor` a uma mensagem.
pub fn set_reaction(conn: &Connection, conv_id: &str, msg_id: &[u8], reactor: &str, emoji: &str) -> Result<(), StorageError> {
    if emoji.is_empty() {
        conn.execute(
            "DELETE FROM reactions WHERE conv_id = ?1 AND msg_id = ?2 AND reactor = ?3",
            params![conv_id, msg_id, reactor],
        )?;
    } else {
        conn.execute(
            "INSERT OR REPLACE INTO reactions (conv_id, msg_id, reactor, emoji) VALUES (?1, ?2, ?3, ?4)",
            params![conv_id, msg_id, reactor, emoji],
        )?;
    }
    Ok(())
}

/// Guarda (ou substitui) um grupo.
pub fn group_put(conn: &Connection, group_id: &str, info: &str, active: bool) -> Result<(), StorageError> {
    conn.execute(
        "INSERT OR REPLACE INTO groups (group_id, info, active) VALUES (?1, ?2, ?3)",
        params![group_id, info, active as i64],
    )?;
    Ok(())
}

/// (info, active) de um grupo.
pub fn group_get(conn: &Connection, group_id: &str) -> Result<Option<(String, bool)>, StorageError> {
    Ok(conn
        .query_row("SELECT info, active FROM groups WHERE group_id = ?1", [group_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0))
        })
        .optional()?)
}

/// Todos os grupos: (group_id, info, active).
pub fn groups_all(conn: &Connection) -> Result<Vec<(String, String, bool)>, StorageError> {
    let mut stmt = conn.prepare("SELECT group_id, info, active FROM groups ORDER BY group_id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? != 0)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn group_delete(conn: &Connection, group_id: &str) -> Result<(), StorageError> {
    conn.execute("DELETE FROM groups WHERE group_id = ?1", [group_id])?;
    Ok(())
}

/// Todas as reacoes de uma conversa: (msg_id, reactor, emoji).
pub fn reactions(conn: &Connection, conv_id: &str) -> Result<Vec<(Vec<u8>, String, String)>, StorageError> {
    let mut stmt = conn.prepare("SELECT msg_id, reactor, emoji FROM reactions WHERE conv_id = ?1 ORDER BY reactor")?;
    let rows = stmt.query_map([conv_id], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Mensagens ENVIADAS a `contact_id` que ainda nao chegaram (por enviar ou so
/// "enviadas" ao relay, sem recibo "entregue"), da mais antiga para a mais
/// recente. Usado quando a sessao com o contacto e reiniciada: estas
/// mensagens foram cifradas com a sessao antiga e tem de ser reenviadas.
pub fn outgoing_undelivered(conn: &Connection, contact_id: &str, limit: u32) -> Result<Vec<StoredMessage>, StorageError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM messages WHERE contact_id = ?1 AND outgoing = 1 AND status < ?2
         ORDER BY seq DESC LIMIT ?3"
    ))?;
    let rows = stmt.query_map(params![contact_id, ST_DELIVERED, limit.clamp(1, 1000) as i64], row_to_msg)?;
    let mut v = rows.collect::<Result<Vec<_>, _>>()?;
    v.reverse();
    Ok(v)
}

/// Volta a por uma mensagem enviada no estado "por enviar" (reenvio).
pub fn reset_outgoing_to_pending(conn: &Connection, contact_id: &str, msg_id: &[u8]) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE messages SET status = ?1 WHERE contact_id = ?2 AND msg_id = ?3 AND outgoing = 1",
        params![ST_PENDING, contact_id, msg_id],
    )?;
    Ok(())
}

/// Apaga da outbox o que ia para `contact_id` (envelopes de uma sessao antiga),
/// menos os de grupos: esses sao cifrados de novo um a um (qt_ffi::groups).
pub fn outbox_delete_contact(conn: &Connection, contact_id: &str) -> Result<(), StorageError> {
    conn.execute("DELETE FROM outbox WHERE contact_id = ?1 AND kind != ?2", params![contact_id, OUTBOX_GROUP])?;
    Ok(())
}

// --------------------------------------------------------------------- outbox

pub fn outbox_put(conn: &Connection, outbox_id: &[u8], contact_id: &str, kind: i64, blob: &[u8]) -> Result<(), StorageError> {
    conn.execute(
        "INSERT INTO outbox (outbox_id, contact_id, kind, blob) VALUES (?1, ?2, ?3, ?4)",
        params![outbox_id, contact_id, kind, blob],
    )?;
    Ok(())
}

/// Remove a entrada e devolve-a (None se ja nao existia).
pub fn outbox_take(conn: &Connection, outbox_id: &[u8]) -> Result<Option<OutboxEntry>, StorageError> {
    let e = conn
        .query_row(
            "SELECT outbox_id, contact_id, kind, blob FROM outbox WHERE outbox_id = ?1",
            [outbox_id],
            |r| Ok(OutboxEntry { outbox_id: r.get(0)?, contact_id: r.get(1)?, kind: r.get(2)?, blob: r.get(3)? }),
        )
        .optional()?;
    if e.is_some() {
        conn.execute("DELETE FROM outbox WHERE outbox_id = ?1", [outbox_id])?;
    }
    Ok(e)
}

/// Tudo o que ainda nao foi confirmado pelo relay, pela ordem de criacao.
pub fn outbox_all(conn: &Connection) -> Result<Vec<OutboxEntry>, StorageError> {
    let mut stmt = conn.prepare("SELECT outbox_id, contact_id, kind, blob FROM outbox ORDER BY seq ASC")?;
    let rows = stmt.query_map([], |r| {
        Ok(OutboxEntry { outbox_id: r.get(0)?, contact_id: r.get(1)?, kind: r.get(2)?, blob: r.get(3)? })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}
