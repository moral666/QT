//! Bloqueio da app com codigo proprio.
//!
//! A chave que abre a base de dados (SQLCipher) passa a ficar cifrada tambem por
//! uma chave derivada do codigo do utilizador (Argon2id). Sem o codigo, a base de
//! dados nao abre, mesmo com acesso ao telemovel desbloqueado.
//!
//! Na app Android, o resultado de [`app_lock_wrap`] e ainda cifrado pelo Android
//! Keystore do aparelho: o ficheiro copiado para outro sitio nao serve para tentar
//! codigos fora do telemovel.
//!
//! Formato (92 bytes):
//! `"QAL1" | m_kib (u32 LE) | t (u32 LE) | p (u32 LE) | sal (16) | nonce (12) | cifra (32) | tag (16)`
//! O cabecalho inteiro (ate ao sal) entra como dados autenticados: alterar os
//! parametros ou o sal faz a decifragem falhar.

use crate::client::QtError;
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use zeroize::{Zeroize, Zeroizing};

const MAGIC: &[u8; 4] = b"QAL1";
const CONTEXT: &[u8] = b"qt/app-lock/v1";
/// Tamanho minimo do codigo (caracteres).
pub const APP_LOCK_MIN_CODE_LEN: u32 = 6;
const MAX_CODE_LEN: usize = 64;
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const HEADER_LEN: usize = 4 + 12 + SALT_LEN;
const BLOB_LEN: usize = HEADER_LEN + NONCE_LEN + KEY_LEN + 16;

// Parametros usados ao cifrar: 64 MiB de memoria, cerca de 0,15 s num PC e
// 0,5 a 1 s num telemovel por tentativa.
const M_KIB: u32 = 64 * 1024;
const T_COST: u32 = 3;
const P_COST: u32 = 1;
// Limites ao decifrar: um ficheiro corrompido nao pode pedir memoria ou tempo absurdos.
const MAX_M_KIB: u32 = 256 * 1024;
const MAX_T_COST: u32 = 10;
const MAX_P_COST: u32 = 4;

fn bad_code() -> QtError {
    QtError::InvalidInput { field: "codigo".into() }
}

fn check_code(code: &str) -> Result<(), QtError> {
    let n = code.chars().count();
    if n < APP_LOCK_MIN_CODE_LEN as usize || n > MAX_CODE_LEN {
        return Err(bad_code());
    }
    Ok(())
}

fn derive(code: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Result<Zeroizing<[u8; KEY_LEN]>, QtError> {
    let params = Params::new(m, t, p, Some(KEY_LEN)).map_err(|_| QtError::Crypto)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    argon.hash_password_into(code.as_bytes(), salt, out.as_mut()).map_err(|_| QtError::Crypto)?;
    Ok(out)
}

fn aad(header: &[u8]) -> Vec<u8> {
    let mut a = CONTEXT.to_vec();
    a.extend_from_slice(header);
    a
}

/// Cifra a chave do armazenamento (32 bytes) com o codigo. Cada chamada usa um
/// sal e um nonce novos.
#[uniffi::export]
pub fn app_lock_wrap(code: String, storage_key: Vec<u8>) -> Result<Vec<u8>, QtError> {
    let mut code = code;
    let mut storage_key = storage_key;
    let result = wrap_inner(&code, &storage_key);
    code.zeroize();
    storage_key.zeroize();
    result
}

fn wrap_inner(code: &str, storage_key: &[u8]) -> Result<Vec<u8>, QtError> {
    check_code(code)?;
    if storage_key.len() != KEY_LEN {
        return Err(QtError::InvalidInput { field: "chave".into() });
    }
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut salt).map_err(|_| QtError::Crypto)?;
    getrandom::getrandom(&mut nonce).map_err(|_| QtError::Crypto)?;

    let mut out = Vec::with_capacity(BLOB_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&M_KIB.to_le_bytes());
    out.extend_from_slice(&T_COST.to_le_bytes());
    out.extend_from_slice(&P_COST.to_le_bytes());
    out.extend_from_slice(&salt);

    let kek = derive(code, &salt, M_KIB, T_COST, P_COST)?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(kek.as_ref()));
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: storage_key, aad: &aad(&out) })
        .map_err(|_| QtError::Crypto)?;
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Devolve a chave do armazenamento. Codigo errado ou dados alterados: `QtError::Crypto`.
#[uniffi::export]
pub fn app_lock_unwrap(code: String, blob: Vec<u8>) -> Result<Vec<u8>, QtError> {
    let mut code = code;
    let result = unwrap_inner(&code, &blob);
    code.zeroize();
    result
}

fn unwrap_inner(code: &str, blob: &[u8]) -> Result<Vec<u8>, QtError> {
    check_code(code)?;
    if blob.len() != BLOB_LEN || &blob[..4] != MAGIC {
        return Err(QtError::InvalidInput { field: "bloqueio".into() });
    }
    let u32_at = |i: usize| u32::from_le_bytes([blob[i], blob[i + 1], blob[i + 2], blob[i + 3]]);
    let (m, t, p) = (u32_at(4), u32_at(8), u32_at(12));
    if m == 0 || m > MAX_M_KIB || t == 0 || t > MAX_T_COST || p == 0 || p > MAX_P_COST {
        return Err(QtError::InvalidInput { field: "bloqueio".into() });
    }
    let header = &blob[..HEADER_LEN];
    let salt = &blob[16..HEADER_LEN];
    let nonce = &blob[HEADER_LEN..HEADER_LEN + NONCE_LEN];
    let ct = &blob[HEADER_LEN + NONCE_LEN..];

    let kek = derive(code, salt, m, t, p)?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(kek.as_ref()));
    let key = cipher
        .decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad: &aad(header) })
        .map_err(|_| QtError::Crypto)?;
    if key.len() != KEY_LEN {
        let mut key = key;
        key.zeroize();
        return Err(QtError::Crypto);
    }
    Ok(key)
}

/// Tamanho minimo do codigo, para a interface validar antes de chamar o Rust.
#[uniffi::export]
pub fn app_lock_min_code_len() -> u32 {
    APP_LOCK_MIN_CODE_LEN
}
