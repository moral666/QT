//! Armazenamento local ENCRIPTADO (SQLCipher) para:
//!   1. A identidade do proprio utilizador (chaves privadas de longo prazo)
//!   2. As sessoes de Double Ratchet com cada contacto (para a conversa
//!      continuar entre execucoes separadas do cliente)
//!
//! A cifra em repouso e feita pelo PROPRIO SQLCipher (a base de dados
//! inteira e cifrada com a passphrase fornecida a `open_database`) - nao
//! reinventamos cifra aqui, so orquestramos onde e como as chaves/sessoes
//! de `core/` sao guardadas.
//!
//! NOTA DE SEGURANCA IMPORTANTE (documentada, nao escondida): nesta fase de
//! desenvolvimento, a passphrase da base de dados e passada como argumento
//! de funcao. Numa app real, esta passphrase NUNCA deve ser um literal no
//! codigo nem escrita em disco em texto plano - deve vir do Android
//! Keystore / iOS Secure Enclave (ver docs/protocol-spec.md secao 5).

pub mod messages;
pub use rusqlite;
use rusqlite::Connection;
use qt_core::primitives::{DhKeyPair, SigningKeyPair};
use qt_core::ratchet::RatchetState;
use thiserror::Error;
use zeroize::Zeroize;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("erro SQLCipher/SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("identidade nao encontrada na base de dados - corre `save_identity` primeiro")]
    IdentityNotFound,
    #[error("dados de identidade corrompidos ou com tamanho invalido")]
    CorruptIdentity,
    #[error("sessao de ratchet guardada esta corrompida (falhou a desserializacao)")]
    CorruptRatchetState,
    #[error("chave de protecao do storage invalida (esperados 32 bytes) ")]
    InvalidStorageKey,
    #[error("base de dados nao abre com esta chave, ou esta corrompida")]
    CannotOpen,
    #[error("secure_delete nao ficou ativo nesta ligacao")]
    SecureDeleteNotActive,
    #[error("prekey inexistente ou ja utilizada")]
    PrekeyNotFoundOrUsed,
    #[error("dados guardados corrompidos ou com tamanho invalido")]
    CorruptData,
    #[cfg(feature = "pq")]
    #[error("chave PQ guardada esta corrompida")]
    CorruptPqKey,
}

/// A identidade completa de um utilizador, tal como persistida localmente.
/// Corresponde as chaves privadas geradas em `core/` no primeiro arranque.
pub struct StoredIdentity {
    pub identity: DhKeyPair,
    pub identity_signing: SigningKeyPair,
    pub signed_pre_key: DhKeyPair,
    pub one_time_pre_key: DhKeyPair,
}

/// Abre (ou cria) a base de dados cifrada em `path`, usando `passphrase`
/// como chave de cifra do SQLCipher, e garante que o schema existe.
///
/// `PRAGMA key` tem de ser a PRIMEIRA operacao na ligacao, antes de
/// qualquer outra query - e assim que o SQLCipher deriva a chave de cifra
/// da base de dados inteira a partir da passphrase.
pub fn open_database(path: &str, passphrase: &str) -> Result<Connection, StorageError> {
    let conn = Connection::open(path)?;

    // PRAGMA key precisa de escaping cuidadoso - usamos o parametro
    // preparado do proprio SQLCipher para evitar problemas de injecao de
    // aspas na passphrase.
    conn.pragma_update(None, "key", passphrase)?;
    init_connection(&conn)?;
    Ok(conn)
}

/// Abre (ou cria) a base de dados cifrada usando uma CHAVE BINARIA de 32
/// bytes (formato "raw key" do SQLCipher, sem KDF de passphrase). E esta a
/// funcao pensada para o Android: a chave de 32 bytes e gerada/protegida
/// pelo Android Keystore (ex.: segredo aleatorio cifrado com uma chave
/// AES-GCM do Keystore) e entregue aqui em memoria; nunca vai para logs,
/// ficheiros nem mensagens de erro.
pub fn open_database_with_key(path: &str, key: &[u8]) -> Result<Connection, StorageError> {
    if key.len() != 32 {
        return Err(StorageError::InvalidStorageKey);
    }
    let conn = Connection::open(path)?;
    let mut hex = String::with_capacity(32 * 2 + 8);
    hex.push_str("PRAGMA key = \"x'");
    for b in key {
        hex.push_str(&format!("{:02x}", b));
    }
    hex.push_str("'\";");
    let r = conn.execute_batch(&hex);
    hex.zeroize();
    r.map_err(|_| StorageError::CannotOpen)?;
    // Qualquer erro aqui (chave errada, ficheiro corrompido) e mapeado para
    // um erro generico, que nunca contem material de chave.
    init_connection(&conn).map_err(|e| match e {
        StorageError::Sqlite(_) => StorageError::CannotOpen,
        other => other,
    })?;
    Ok(conn)
}

fn init_connection(conn: &Connection) -> Result<(), StorageError> {
    // secure_delete garante que paginas apagadas sao sobrescritas com
    // zeros, em vez de simplesmente marcadas como livres (relevante para
    // "mensagens que desaparecem" nao deixarem residuos recuperaveis).
    conn.pragma_update(None, "secure_delete", "ON")?;
    // Confirmar de facto (e nao assumir) que ficou ativo.
    let active: i64 = conn.query_row("PRAGMA secure_delete", [], |r| r.get(0))?;
    if active != 1 {
        return Err(StorageError::SecureDeleteNotActive);
    }

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS identity (
            id INTEGER PRIMARY KEY CHECK (id = 0),
            identity_private BLOB NOT NULL,
            identity_signing_private BLOB NOT NULL,
            signed_pre_key_private BLOB NOT NULL,
            one_time_pre_key_private BLOB NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sessions (
            contact_id TEXT PRIMARY KEY,
            ratchet_state BLOB NOT NULL,
            updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
        );
        CREATE TABLE IF NOT EXISTS one_time_prekeys (
            id INTEGER PRIMARY KEY,
            private BLOB NOT NULL
        );
        CREATE TABLE IF NOT EXISTS pq_prekeys (
            id INTEGER PRIMARY KEY,
            private BLOB NOT NULL,
            created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
        );
        CREATE TABLE IF NOT EXISTS noise_static (
            id INTEGER PRIMARY KEY CHECK (id = 0),
            private BLOB NOT NULL
        );
        CREATE TABLE IF NOT EXISTS session_meta (
            contact_id TEXT PRIMARY KEY,
            peer_identity_public BLOB NOT NULL,
            protocol TEXT NOT NULL,
            last_recv_seq INTEGER NOT NULL DEFAULT 0,
            send_seq INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS kv (
            k TEXT PRIMARY KEY,
            v BLOB NOT NULL
        );
        CREATE TABLE IF NOT EXISTS seen_envelopes (
            contact_id TEXT NOT NULL,
            envelope_id BLOB NOT NULL,
            seen_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
            PRIMARY KEY (contact_id, envelope_id)
        );
        -- Envelopes de contactos APAGADOS: so um hash (sem o contact_id em claro),
        -- para que o relay nao consiga reentregar mensagens antigas e recriar a sessao.
        CREATE TABLE IF NOT EXISTS seen_tombstones (
            digest BLOB PRIMARY KEY
        );",
    )?;
    // Historico de mensagens e outbox (ver messages.rs): o texto passou a ser
    // guardado, SEMPRE dentro desta base cifrada e na mesma transacao que o
    // avanco do ratchet, para nunca se perder uma mensagem ja decifrada.
    conn.execute_batch(messages::SCHEMA)?;
    messages::migrate(conn)?;
    Ok(())
}

/// Guarda a identidade do utilizador (so pode existir uma - tabela com
/// `id = 0` fixo). Chamar uma unica vez, no primeiro arranque do cliente.
pub fn save_identity(conn: &Connection, identity: &StoredIdentity) -> Result<(), StorageError> {
    conn.execute(
        "INSERT OR REPLACE INTO identity
            (id, identity_private, identity_signing_private, signed_pre_key_private, one_time_pre_key_private)
         VALUES (0, ?1, ?2, ?3, ?4)",
        rusqlite::params![
            identity.identity.private.to_bytes().to_vec(),
            identity.identity_signing.to_bytes().to_vec(),
            identity.signed_pre_key.private.to_bytes().to_vec(),
            identity.one_time_pre_key.private.to_bytes().to_vec(),
        ],
    )?;
    Ok(())
}

pub fn load_identity(conn: &Connection) -> Result<StoredIdentity, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT identity_private, identity_signing_private, signed_pre_key_private, one_time_pre_key_private
         FROM identity WHERE id = 0",
    )?;

    let row = stmt
        .query_row([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StorageError::IdentityNotFound,
            other => StorageError::Sqlite(other),
        })?;

    let (identity_private, identity_signing_private, signed_pre_key_private, one_time_pre_key_private) =
        row;

    let to_array = |v: &[u8]| -> Result<[u8; 32], StorageError> {
        v.try_into().map_err(|_| StorageError::CorruptIdentity)
    };

    Ok(StoredIdentity {
        identity: DhKeyPair {
            private: x25519_dalek::StaticSecret::from(to_array(&identity_private)?),
            public: x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(to_array(
                &identity_private,
            )?)),
        },
        identity_signing: SigningKeyPair::from_bytes(&to_array(&identity_signing_private)?),
        signed_pre_key: DhKeyPair {
            private: x25519_dalek::StaticSecret::from(to_array(&signed_pre_key_private)?),
            public: x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(to_array(
                &signed_pre_key_private,
            )?)),
        },
        one_time_pre_key: DhKeyPair {
            private: x25519_dalek::StaticSecret::from(to_array(&one_time_pre_key_private)?),
            public: x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(to_array(
                &one_time_pre_key_private,
            )?)),
        },
    })
}

/// Guarda (ou substitui) a sessao de Double Ratchet com um contacto.
/// Chamar depois de cada mensagem enviada/recebida, para o estado do
/// ratchet nunca ficar desatualizado em disco face ao que esta em memoria.
pub fn save_session(
    conn: &Connection,
    contact_id: &str,
    ratchet: &RatchetState,
) -> Result<(), StorageError> {
    conn.execute(
        "INSERT OR REPLACE INTO sessions (contact_id, ratchet_state, updated_at)
         VALUES (?1, ?2, strftime('%s','now'))",
        rusqlite::params![contact_id, ratchet.to_bytes()],
    )?;
    Ok(())
}

pub fn load_session(conn: &Connection, contact_id: &str) -> Result<Option<RatchetState>, StorageError> {
    let mut stmt = conn.prepare("SELECT ratchet_state FROM sessions WHERE contact_id = ?1")?;
    let bytes: Option<Vec<u8>> = stmt
        .query_row([contact_id], |row| row.get(0))
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;

    match bytes {
        Some(b) => {
            let state = RatchetState::from_bytes(&b).ok_or(StorageError::CorruptRatchetState)?;
            Ok(Some(state))
        }
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Prekeys de uso unico
// ---------------------------------------------------------------------------

fn dh_from_private(bytes: &[u8]) -> Result<DhKeyPair, StorageError> {
    let arr: [u8; 32] = bytes.try_into().map_err(|_| StorageError::CorruptData)?;
    let private = x25519_dalek::StaticSecret::from(arr);
    let public = x25519_dalek::PublicKey::from(&private);
    Ok(DhKeyPair { private, public })
}

pub fn save_one_time_prekey(conn: &Connection, id: u32, key: &DhKeyPair) -> Result<(), StorageError> {
    conn.execute(
        "INSERT OR REPLACE INTO one_time_prekeys (id, private) VALUES (?1, ?2)",
        rusqlite::params![id, key.private.to_bytes().to_vec()],
    )?;
    Ok(())
}

/// Devolve a one-time prekey e APAGA-A (com secure_delete) numa unica
/// transacao: uma prekey so pode ser consumida uma vez. Uma segunda tentativa
/// (reutilizacao) devolve `PrekeyNotFoundOrUsed`.
pub fn take_one_time_prekey(conn: &mut Connection, id: u32) -> Result<DhKeyPair, StorageError> {
    let tx = conn.transaction()?;
    let bytes: Vec<u8> = tx
        .query_row("SELECT private FROM one_time_prekeys WHERE id = ?1", [id], |r| r.get(0))
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StorageError::PrekeyNotFoundOrUsed,
            other => StorageError::Sqlite(other),
        })?;
    tx.execute("DELETE FROM one_time_prekeys WHERE id = ?1", [id])?;
    let kp = dh_from_private(&bytes)?;
    tx.commit()?;
    Ok(kp)
}

pub fn count_one_time_prekeys(conn: &Connection) -> Result<u32, StorageError> {
    Ok(conn.query_row("SELECT COUNT(*) FROM one_time_prekeys", [], |r| r.get::<_, u32>(0))?)
}

// ---------------------------------------------------------------------------
// Prekeys PQ (ML-KEM-768) - feature "pq"
// ---------------------------------------------------------------------------

#[cfg(feature = "pq")]
pub fn save_pq_prekey(
    conn: &Connection,
    id: u32,
    key: &qt_core::pqxdh::PqPreKeyPair,
) -> Result<(), StorageError> {
    let mut blob = key.to_bytes();
    let r = conn.execute(
        "INSERT OR REPLACE INTO pq_prekeys (id, private) VALUES (?1, ?2)",
        rusqlite::params![id, blob],
    );
    blob.zeroize();
    r?;
    Ok(())
}

#[cfg(feature = "pq")]
pub fn load_pq_prekey(
    conn: &Connection,
    id: u32,
) -> Result<Option<qt_core::pqxdh::PqPreKeyPair>, StorageError> {
    let blob: Option<Vec<u8>> = conn
        .query_row("SELECT private FROM pq_prekeys WHERE id = ?1", [id], |r| r.get(0))
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    match blob {
        None => Ok(None),
        Some(mut b) => {
            let r = qt_core::pqxdh::PqPreKeyPair::from_bytes(&b).map_err(|_| StorageError::CorruptPqKey);
            b.zeroize();
            r.map(Some)
        }
    }
}

#[cfg(feature = "pq")]
pub fn delete_pq_prekey(conn: &Connection, id: u32) -> Result<(), StorageError> {
    conn.execute("DELETE FROM pq_prekeys WHERE id = ?1", [id])?;
    Ok(())
}

/// Maior numero de prekey PQ guardado (0 se nao houver nenhuma).
#[cfg(feature = "pq")]
pub fn max_pq_prekey_id(conn: &Connection) -> Result<u32, StorageError> {
    Ok(conn.query_row("SELECT COALESCE(MAX(id), 0) FROM pq_prekeys", [], |r| r.get::<_, u32>(0))?)
}

/// Apaga as prekeys PQ criadas antes de `before_unix`, exceto `keep_id` (a atual).
#[cfg(feature = "pq")]
pub fn delete_pq_prekeys_before(conn: &Connection, before_unix: i64, keep_id: u32) -> Result<usize, StorageError> {
    Ok(conn.execute("DELETE FROM pq_prekeys WHERE created_at < ?1 AND id != ?2", rusqlite::params![before_unix, keep_id])?)
}

// ---------------------------------------------------------------------------
// Chave estatica Noise do cliente
// ---------------------------------------------------------------------------

pub fn save_noise_static(conn: &Connection, private: &[u8]) -> Result<(), StorageError> {
    if private.len() != 32 {
        return Err(StorageError::CorruptData);
    }
    conn.execute(
        "INSERT OR REPLACE INTO noise_static (id, private) VALUES (0, ?1)",
        [private],
    )?;
    Ok(())
}

pub fn load_noise_static(conn: &Connection) -> Result<Option<Vec<u8>>, StorageError> {
    let v: Option<Vec<u8>> = conn
        .query_row("SELECT private FROM noise_static WHERE id = 0", [], |r| r.get(0))
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    if let Some(ref b) = v {
        if b.len() != 32 {
            return Err(StorageError::CorruptData);
        }
    }
    Ok(v)
}

// ---------------------------------------------------------------------------
// Metadados de sessao e deduplicacao de envelopes (replay)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    pub contact_id: String,
    pub peer_identity_public: Vec<u8>,
    /// "x3dh" ou "pqxdh" - protocolo usado para estabelecer a sessao.
    pub protocol: String,
    pub last_recv_seq: u64,
    pub send_seq: u64,
}

pub fn save_session_meta(conn: &Connection, m: &SessionMeta) -> Result<(), StorageError> {
    if m.peer_identity_public.len() != 32 {
        return Err(StorageError::CorruptData);
    }
    conn.execute(
        "INSERT OR REPLACE INTO session_meta
            (contact_id, peer_identity_public, protocol, last_recv_seq, send_seq)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            m.contact_id,
            m.peer_identity_public,
            m.protocol,
            m.last_recv_seq as i64,
            m.send_seq as i64
        ],
    )?;
    Ok(())
}

pub fn load_session_meta(conn: &Connection, contact_id: &str) -> Result<Option<SessionMeta>, StorageError> {
    let row = conn.query_row(
        "SELECT peer_identity_public, protocol, last_recv_seq, send_seq
         FROM session_meta WHERE contact_id = ?1",
        [contact_id],
        |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
    );
    match row {
        Ok((peer, protocol, rs, ss)) => {
            if peer.len() != 32 || rs < 0 || ss < 0 {
                return Err(StorageError::CorruptData);
            }
            Ok(Some(SessionMeta {
                contact_id: contact_id.to_string(),
                peer_identity_public: peer,
                protocol,
                last_recv_seq: rs as u64,
                send_seq: ss as u64,
            }))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Regista um envelope como visto. Devolve `true` se for NOVO e `false` se ja
/// tinha sido visto (duplicado/replay) - o chamador deve descartar duplicados
/// ANTES de os entregar ao Double Ratchet.
pub fn record_envelope(conn: &Connection, contact_id: &str, envelope_id: &[u8]) -> Result<bool, StorageError> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO seen_envelopes (contact_id, envelope_id) VALUES (?1, ?2)",
        rusqlite::params![contact_id, envelope_id],
    )?;
    Ok(n == 1)
}

/// Apaga registos de deduplicacao mais antigos que `older_than_secs`.
pub fn prune_seen_envelopes(conn: &Connection, older_than_secs: i64) -> Result<usize, StorageError> {
    Ok(conn.execute(
        "DELETE FROM seen_envelopes WHERE seen_at < strftime('%s','now') - ?1",
        [older_than_secs],
    )?)
}

pub fn delete_session(conn: &Connection, contact_id: &str) -> Result<(), StorageError> {
    conn.execute("DELETE FROM sessions WHERE contact_id = ?1", [contact_id])?;
    conn.execute("DELETE FROM session_meta WHERE contact_id = ?1", [contact_id])?;
    tombstone_seen_envelopes(conn, contact_id)?;
    Ok(())
}

fn tombstone_digest(contact_id: &str, envelope_id: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"QT-SEEN-TOMBSTONE-v1");
    h.update((contact_id.len() as u32).to_be_bytes());
    h.update(contact_id.as_bytes());
    h.update(envelope_id);
    h.finalize().to_vec()
}

/// Troca os registos anti-replay de um contacto por hashes anonimos e apaga-os.
/// O contacto deixa de aparecer em claro na base de dados, mas um envelope
/// antigo reentregue pelo relay continua a ser reconhecido como repetido.
pub fn tombstone_seen_envelopes(conn: &Connection, contact_id: &str) -> Result<(), StorageError> {
    let ids: Vec<Vec<u8>> = {
        let mut stmt = conn.prepare("SELECT envelope_id FROM seen_envelopes WHERE contact_id = ?1")?;
        let rows = stmt.query_map([contact_id], |r| r.get::<_, Vec<u8>>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    {
        let mut ins = conn.prepare("INSERT OR IGNORE INTO seen_tombstones (digest) VALUES (?1)")?;
        for id in &ids {
            ins.execute([tombstone_digest(contact_id, id)])?;
        }
    }
    conn.execute("DELETE FROM seen_envelopes WHERE contact_id = ?1", [contact_id])?;
    Ok(())
}

/// Apaga TODA a informacao sensivel (identidade, prekeys, sessoes, Noise).
/// Com secure_delete ON as paginas sao sobrescritas. Usar em "apagar conta".
pub fn wipe_all(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "DELETE FROM identity; DELETE FROM sessions; DELETE FROM one_time_prekeys;
         DELETE FROM pq_prekeys; DELETE FROM noise_static; DELETE FROM session_meta;
         DELETE FROM seen_envelopes; DELETE FROM seen_tombstones; DELETE FROM kv;
         DELETE FROM messages; DELETE FROM outbox; DELETE FROM reactions; DELETE FROM groups; VACUUM;",
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Auxiliares: kv generico, leitura nao-destrutiva de prekeys, dedup read-only
// ---------------------------------------------------------------------------

pub fn kv_set(conn: &Connection, key: &str, value: &[u8]) -> Result<(), StorageError> {
    conn.execute("INSERT OR REPLACE INTO kv (k, v) VALUES (?1, ?2)", rusqlite::params![key, value])?;
    Ok(())
}

pub fn kv_get(conn: &Connection, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
    match conn.query_row("SELECT v FROM kv WHERE k = ?1", [key], |r| r.get::<_, Vec<u8>>(0)) {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn kv_delete(conn: &Connection, key: &str) -> Result<(), StorageError> {
    conn.execute("DELETE FROM kv WHERE k = ?1", [key])?;
    Ok(())
}

/// Le uma one-time prekey SEM a consumir (usar `delete_one_time_prekey`
/// so depois de o handshake ter sido validado com sucesso).
pub fn load_one_time_prekey(conn: &Connection, id: u32) -> Result<DhKeyPair, StorageError> {
    let bytes: Vec<u8> = conn
        .query_row("SELECT private FROM one_time_prekeys WHERE id = ?1", [id], |r| r.get(0))
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StorageError::PrekeyNotFoundOrUsed,
            other => StorageError::Sqlite(other),
        })?;
    dh_from_private(&bytes)
}

pub fn delete_one_time_prekey(conn: &Connection, id: u32) -> Result<(), StorageError> {
    conn.execute("DELETE FROM one_time_prekeys WHERE id = ?1", [id])?;
    Ok(())
}

/// Apaga as one-time prekeys com numero abaixo de `id` (lotes antigos que ja nao
/// estao no relay e cujo prazo para uma primeira mensagem passou).
pub fn delete_one_time_prekeys_below(conn: &Connection, id: u32) -> Result<usize, StorageError> {
    Ok(conn.execute("DELETE FROM one_time_prekeys WHERE id < ?1", [id])?)
}

/// Consulta (sem registar) se um envelope ja foi processado.
pub fn is_envelope_seen(conn: &Connection, contact_id: &str, envelope_id: &[u8]) -> Result<bool, StorageError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM seen_envelopes WHERE contact_id = ?1 AND envelope_id = ?2",
        rusqlite::params![contact_id, envelope_id],
        |r| r.get(0),
    )?;
    if n > 0 {
        return Ok(true);
    }
    let t: i64 = conn.query_row(
        "SELECT COUNT(*) FROM seen_tombstones WHERE digest = ?1",
        [tombstone_digest(contact_id, envelope_id)],
        |r| r.get(0),
    )?;
    Ok(t > 0)
}

pub fn identity_exists(conn: &Connection) -> Result<bool, StorageError> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM identity WHERE id = 0", [], |r| r.get(0))?;
    Ok(n > 0)
}
