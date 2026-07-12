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

use rusqlite::Connection;
use qt_core::primitives::{DhKeyPair, SigningKeyPair};
use qt_core::ratchet::RatchetState;
use thiserror::Error;

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

    // secure_delete garante que paginas apagadas sao sobrescritas com
    // zeros, em vez de simplesmente marcadas como livres (relevante para
    // "mensagens que desaparecem" nao deixarem residuos recuperaveis).
    conn.pragma_update(None, "secure_delete", "ON")?;

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
        );",
    )?;

    Ok(conn)
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
