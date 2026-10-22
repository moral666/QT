//! Recuperacao da conta (desde a app 0.9.19): 12 palavras abrem uma copia cifrada que fica
//! no relay. O relay guarda bytes que nao consegue ler; so quem tem as palavras os abre.
//!
//!   * As palavras sao 128 bits aleatorios escritos com a lista BIP-39 inglesa (12 palavras,
//!     a ultima leva uma verificacao: um erro de escrita e quase sempre apanhado).
//!   * Argon2id (64 MiB, 3 passagens) transforma-as em 64 bytes: a CHAVE da copia (nunca sai
//!     do telemovel) e o IDENTIFICADOR da copia no relay. O Argon2 torna lento tentar adivinhar.
//!   * Cifra: XChaCha20-Poly1305 com nonce aleatorio (a copia e refeita varias vezes com a
//!     mesma chave), com enchimento ate um multiplo de 4096 bytes (o tamanho nao diz quantos
//!     contactos ha).
//!
//! FORMATO: `[1 versao=1][24 nonce][cifrado + etiqueta 16]`. Dentro: `[4 tamanho BE][conteudo][zeros]`.

use argon2::{Algorithm, Argon2, Params, Version};
use bip39::{Language, Mnemonic};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

pub const VERSION: u8 = 1;
/// Palavras da chave de recuperacao.
pub const WORDS: usize = 12;
const ENTROPY_LEN: usize = 16;
const SALT: &[u8] = b"SEALER-RECOVERY-v1";
/// Memoria do Argon2id, em KiB (64 MiB).
const MEMORY_KIB: u32 = 64 * 1024;
const PASSES: u32 = 3;
const PAD: usize = 4096;
/// Maior conteudo aceite (antes do enchimento).
pub const MAX_PLAINTEXT: usize = 256 * 1024;
/// Maior copia cifrada.
pub const MAX_SEALED: usize = 1 + 24 + 4 + MAX_PLAINTEXT + PAD + 16;

#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryError {
    /// Palavras desconhecidas, a mais ou a menos, ou a verificacao falhou.
    BadWords,
    TooLarge,
    /// Chave errada ou copia alterada/estragada.
    BadBackup,
    Kdf,
}

/// Chave e identificador derivados das palavras.
pub struct RecoveryKeys {
    pub key: Zeroizing<[u8; 32]>,
    /// Identificador da copia no relay (64 caracteres hex).
    pub lookup: String,
}

/// 128 bits aleatorios novos (o segredo de onde saem as palavras).
pub fn new_entropy() -> Zeroizing<[u8; ENTROPY_LEN]> {
    let mut e = Zeroizing::new([0u8; ENTROPY_LEN]);
    OsRng.fill_bytes(e.as_mut());
    e
}

/// As 12 palavras deste segredo.
pub fn words_of(entropy: &[u8; ENTROPY_LEN]) -> Vec<String> {
    let m = Mnemonic::from_entropy_in(Language::English, entropy).expect("16 bytes dao 12 palavras");
    m.words().map(str::to_string).collect()
}

/// O segredo a partir das palavras (aceita maiusculas e espacos a mais).
pub fn entropy_of(words: &[String]) -> Result<Zeroizing<[u8; ENTROPY_LEN]>, RecoveryError> {
    if words.len() != WORDS {
        return Err(RecoveryError::BadWords);
    }
    let phrase = Zeroizing::new(words.iter().map(|w| w.trim().to_lowercase()).collect::<Vec<_>>().join(" "));
    let m = Mnemonic::parse_in_normalized(Language::English, &phrase).map_err(|_| RecoveryError::BadWords)?;
    let (bytes, len) = m.to_entropy_array();
    if len != ENTROPY_LEN {
        return Err(RecoveryError::BadWords);
    }
    let mut out = Zeroizing::new([0u8; ENTROPY_LEN]);
    out.copy_from_slice(&bytes[..ENTROPY_LEN]);
    Ok(out)
}

/// Palavra da lista? (para a app avisar logo de uma palavra mal escrita).
pub fn is_word(w: &str) -> bool {
    Language::English.find_word(&w.trim().to_lowercase()).is_some()
}

/// Argon2id: chave da copia e identificador no relay. Demora ~1 s num telemovel (chamar fora
/// da thread principal).
pub fn derive(entropy: &[u8; ENTROPY_LEN]) -> Result<RecoveryKeys, RecoveryError> {
    derive_with(entropy, MEMORY_KIB, PASSES)
}

/// SO PARA TESTES: a mesma derivacao com Argon2 minimo (rapida e fraca). Nunca usar nas apps.
#[doc(hidden)]
pub fn derive_fast_for_tests(entropy: &[u8; ENTROPY_LEN]) -> Result<RecoveryKeys, RecoveryError> {
    derive_with(entropy, 64, 1)
}

pub(crate) fn derive_with(entropy: &[u8; ENTROPY_LEN], memory_kib: u32, passes: u32) -> Result<RecoveryKeys, RecoveryError> {
    let params = Params::new(memory_kib, passes, 1, Some(64)).map_err(|_| RecoveryError::Kdf)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = Zeroizing::new([0u8; 64]);
    argon.hash_password_into(entropy, SALT, out.as_mut()).map_err(|_| RecoveryError::Kdf)?;
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&out[..32]);
    let lookup = out[32..].iter().map(|b| format!("{b:02x}")).collect();
    Ok(RecoveryKeys { key, lookup })
}

/// Cifra a copia com a chave derivada das palavras.
pub fn seal(key: &[u8; 32], content: &[u8]) -> Result<Vec<u8>, RecoveryError> {
    if content.len() > MAX_PLAINTEXT {
        return Err(RecoveryError::TooLarge);
    }
    let padded = (4 + content.len()).div_ceil(PAD) * PAD;
    let mut plain = Zeroizing::new(vec![0u8; padded]);
    plain[..4].copy_from_slice(&(content.len() as u32).to_be_bytes());
    plain[4..4 + content.len()].copy_from_slice(content);
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: &plain, aad: &[VERSION] })
        .map_err(|_| RecoveryError::BadBackup)?;
    let mut out = Vec::with_capacity(1 + 24 + ct.len());
    out.push(VERSION);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Abre a copia. Chave errada ou bytes alterados: `BadBackup`.
pub fn open(key: &[u8; 32], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, RecoveryError> {
    if sealed.len() < 1 + 24 + 16 + 4 || sealed.len() > MAX_SEALED || sealed[0] != VERSION {
        return Err(RecoveryError::BadBackup);
    }
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let plain = Zeroizing::new(
        cipher
            .decrypt(XNonce::from_slice(&sealed[1..25]), Payload { msg: &sealed[25..], aad: &[VERSION] })
            .map_err(|_| RecoveryError::BadBackup)?,
    );
    let len = u32::from_be_bytes(plain[..4].try_into().map_err(|_| RecoveryError::BadBackup)?) as usize;
    if 4 + len > plain.len() {
        return Err(RecoveryError::BadBackup);
    }
    Ok(Zeroizing::new(plain[4..4 + len].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast(e: &[u8; 16]) -> RecoveryKeys {
        derive_with(e, 64, 1).unwrap()
    }

    #[test]
    fn palavras_ida_e_volta_e_erros_de_escrita() {
        let e = new_entropy();
        let w = words_of(&e);
        assert_eq!(w.len(), WORDS);
        assert!(w.iter().all(|x| is_word(x)));
        assert_eq!(*entropy_of(&w).unwrap(), *e);
        // Maiusculas e espacos a mais: aceites.
        let messy: Vec<String> = w.iter().map(|x| format!("  {}  ", x.to_uppercase())).collect();
        assert_eq!(*entropy_of(&messy).unwrap(), *e);
        // Palavras trocadas de lugar, palavra inventada ou a menos: recusado (quase sempre).
        let mut swapped = w.clone();
        swapped.swap(0, 11);
        if swapped != w {
            assert!(entropy_of(&swapped).is_err() || *entropy_of(&swapped).unwrap() != *e);
        }
        let mut bad = w.clone();
        bad[3] = "sealerx".into();
        assert_eq!(entropy_of(&bad).err(), Some(RecoveryError::BadWords));
        assert_eq!(entropy_of(&w[..11]).err(), Some(RecoveryError::BadWords));
        assert!(!is_word("sealerx"));
    }

    #[test]
    fn vetor_conhecido_da_lista_bip39() {
        // Vetor oficial BIP-39: 16 bytes a zeros.
        let w = words_of(&[0u8; 16]);
        assert_eq!(w.join(" "), "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about");
    }

    #[test]
    fn derivacao_deterministica_e_separada() {
        let a = fast(&[1u8; 16]);
        let b = fast(&[1u8; 16]);
        let c = fast(&[2u8; 16]);
        assert_eq!((*a.key, a.lookup.clone()), (*b.key, b.lookup.clone()));
        assert_ne!(a.lookup, c.lookup);
        assert_eq!(a.lookup.len(), 64);
        assert_ne!(a.lookup, a.key.iter().map(|b| format!("{b:02x}")).collect::<String>());
    }

    #[test]
    fn cifrar_abrir_e_recusar() {
        let k = fast(&[3u8; 16]);
        let s = seal(&k.key, b"contactos").unwrap();
        assert_eq!(s.len(), 1 + 24 + 4096 + 16);
        assert_eq!(&**open(&k.key, &s).unwrap(), b"contactos");
        assert_ne!(seal(&k.key, b"contactos").unwrap(), s, "nonce aleatorio");
        let other = fast(&[4u8; 16]);
        assert_eq!(open(&other.key, &s).err(), Some(RecoveryError::BadBackup));
        let mut m = s.clone();
        m[30] ^= 1;
        assert_eq!(open(&k.key, &m).err(), Some(RecoveryError::BadBackup));
        assert_eq!(seal(&k.key, &vec![0u8; MAX_PLAINTEXT + 1]).err(), Some(RecoveryError::TooLarge));
        let big = seal(&k.key, &vec![9u8; MAX_PLAINTEXT]).unwrap();
        assert!(big.len() <= MAX_SEALED);
    }
}
