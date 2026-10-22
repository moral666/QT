//! Denuncias: cifradas na app para a CHAVE DE OPERADOR (o Nuno), guardadas pelo relay
//! como bytes opacos e abertas so no computador do operador (`cargo run --example denuncias`).
//!
//!   * O operador tem UMA semente de 32 bytes (fora do git e do servidor). Dela saem:
//!       - a chave X25519 das denuncias (a publica esta fixa em `OPERATOR_REPORT_PUBLIC`,
//!         igual no Android e no iPhone);
//!       - a chave Ed25519 com que a ferramenta se autentica no relay; o relay so aceita
//!         ordens de operador (listar, apagar, banir) dessa chave (`/etc/qt/operator.pub`).
//!   * Cifra estilo "sealed box": chave X25519 efemera por denuncia, HKDF-SHA256 e
//!     ChaCha20-Poly1305. O relay nao consegue ler nem alterar a denuncia.
//!   * O conteudo vai com enchimento ate um multiplo de 1024 bytes (o tamanho nao diz
//!     quantas mensagens foram incluidas).
//!
//! FORMATO: `[1 versao=1][32 publica efemera][cifrado + etiqueta 16]`.
//! Dentro: `[4 tamanho BE][conteudo][zeros]`.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use rand_core::OsRng;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

/// Chave publica X25519 das denuncias. Gerada pelo Nuno com
/// `cargo run -p qt_server --example denuncias -- chave`. Com zeros a app nao envia denuncias.
pub const OPERATOR_REPORT_PUBLIC: [u8; 32] = [0xb3, 0x8f, 0x39, 0x87, 0x26, 0xc9, 0x39, 0x6f, 0x07, 0x88, 0xe7, 0x26, 0xd6, 0x4f, 0xa1, 0x84, 0xe9, 0xa4, 0x79, 0xa5, 0x1e, 0x2e, 0x91, 0xfc, 0x1b, 0xe5, 0x47, 0x5c, 0x22, 0x86, 0xae, 0x66];

pub const VERSION: u8 = 1;
/// Maior denuncia cifrada aceite (app e relay).
pub const MAX_SEALED: usize = 12_288;
/// Maior conteudo (antes do enchimento) que cabe em `MAX_SEALED`.
pub const MAX_PLAINTEXT: usize = MAX_SEALED - 1 - 32 - 16 - 4;
const PAD: usize = 1024;
const INFO: &[u8] = b"SEALER-REPORT-v1";

#[derive(Debug, PartialEq, Eq)]
pub enum ReportError {
    /// Chave do operador por preencher (zeros).
    NoOperatorKey,
    TooLarge,
    BadFormat,
}

fn derive(seed: &[u8; 32], label: &[u8]) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(Some(b"SEALER-OPERATOR"), seed);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(label, out.as_mut()).expect("32 bytes cabem no HKDF");
    out
}

/// Chave privada X25519 das denuncias (so o operador).
pub fn operator_report_secret(seed: &[u8; 32]) -> StaticSecret {
    StaticSecret::from(*derive(seed, b"report-x25519"))
}

/// Chave publica das denuncias (o que vai para `OPERATOR_REPORT_PUBLIC`).
pub fn operator_report_public(seed: &[u8; 32]) -> [u8; 32] {
    PublicKey::from(&operator_report_secret(seed)).to_bytes()
}

/// Chave privada Ed25519 com que a ferramenta do operador se autentica no relay.
pub fn operator_signing_private(seed: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    derive(seed, b"relay-ed25519")
}

/// Chave publica Ed25519 do operador (o que vai para `/etc/qt/operator.pub`, em hex).
pub fn operator_signing_public(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(&operator_signing_private(seed)).verifying_key().to_bytes()
}

fn aead_key(shared: &[u8; 32], eph_pub: &[u8; 32], op_pub: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let salt = [eph_pub.as_slice(), op_pub.as_slice()].concat();
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared);
    let mut k = Zeroizing::new([0u8; 32]);
    hk.expand(INFO, k.as_mut()).expect("32 bytes cabem no HKDF");
    k
}

/// Cifra `content` para a chave publica `op_pub` do operador.
pub fn seal_with(op_pub: &[u8; 32], content: &[u8]) -> Result<Vec<u8>, ReportError> {
    if op_pub == &[0u8; 32] {
        return Err(ReportError::NoOperatorKey);
    }
    if content.len() > MAX_PLAINTEXT {
        return Err(ReportError::TooLarge);
    }
    let padded_len = (4 + content.len()).div_ceil(PAD) * PAD;
    let mut plain = Zeroizing::new(vec![0u8; padded_len.min(MAX_PLAINTEXT + 4)]);
    plain[..4].copy_from_slice(&(content.len() as u32).to_be_bytes());
    plain[4..4 + content.len()].copy_from_slice(content);

    let eph = StaticSecret::random_from_rng(OsRng);
    let eph_pub = PublicKey::from(&eph).to_bytes();
    let shared = Zeroizing::new(eph.diffie_hellman(&PublicKey::from(*op_pub)).to_bytes());
    let key = aead_key(&shared, &eph_pub, op_pub);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_ref()));
    // Chave unica por denuncia (efemera), por isso o nonce fixo nao se repete com a mesma chave.
    let ct = cipher
        .encrypt(Nonce::from_slice(&[0u8; 12]), Payload { msg: &plain, aad: &[VERSION] })
        .map_err(|_| ReportError::BadFormat)?;
    let mut out = Vec::with_capacity(1 + 32 + ct.len());
    out.push(VERSION);
    out.extend_from_slice(&eph_pub);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Cifra para a chave fixa do operador.
pub fn seal(content: &[u8]) -> Result<Vec<u8>, ReportError> {
    seal_with(&OPERATOR_REPORT_PUBLIC, content)
}

/// So o operador: abre uma denuncia com a semente.
pub fn open(seed: &[u8; 32], sealed: &[u8]) -> Result<Vec<u8>, ReportError> {
    if sealed.len() < 1 + 32 + 16 + 4 || sealed.len() > MAX_SEALED || sealed[0] != VERSION {
        return Err(ReportError::BadFormat);
    }
    let secret = operator_report_secret(seed);
    let op_pub = PublicKey::from(&secret).to_bytes();
    let eph_pub: [u8; 32] = sealed[1..33].try_into().map_err(|_| ReportError::BadFormat)?;
    let shared = Zeroizing::new(secret.diffie_hellman(&PublicKey::from(eph_pub)).to_bytes());
    let key = aead_key(&shared, &eph_pub, &op_pub);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_ref()));
    let plain = Zeroizing::new(
        cipher
            .decrypt(Nonce::from_slice(&[0u8; 12]), Payload { msg: &sealed[33..], aad: &[VERSION] })
            .map_err(|_| ReportError::BadFormat)?,
    );
    let len = u32::from_be_bytes(plain[..4].try_into().map_err(|_| ReportError::BadFormat)?) as usize;
    if 4 + len > plain.len() {
        return Err(ReportError::BadFormat);
    }
    Ok(plain[4..4 + len].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cifrar_e_abrir() {
        let seed = [3u8; 32];
        let op = operator_report_public(&seed);
        let s = seal_with(&op, b"denuncia de teste").unwrap();
        assert_eq!(s.len(), 1 + 32 + 1024 + 16);
        assert_eq!(open(&seed, &s).unwrap(), b"denuncia de teste");
        // Outra semente, bytes mexidos ou versao errada: recusado.
        assert_eq!(open(&[4u8; 32], &s), Err(ReportError::BadFormat));
        let mut m = s.clone();
        m[40] ^= 1;
        assert_eq!(open(&seed, &m), Err(ReportError::BadFormat));
        let mut m = s.clone();
        m[0] = 2;
        assert_eq!(open(&seed, &m), Err(ReportError::BadFormat));
    }

    #[test]
    fn tamanhos_e_chave_por_preencher() {
        let seed = [5u8; 32];
        let op = operator_report_public(&seed);
        let big = vec![7u8; MAX_PLAINTEXT];
        let s = seal_with(&op, &big).unwrap();
        assert!(s.len() <= MAX_SEALED);
        assert_eq!(open(&seed, &s).unwrap(), big);
        assert_eq!(seal_with(&op, &vec![0u8; MAX_PLAINTEXT + 1]), Err(ReportError::TooLarge));
        assert_eq!(seal_with(&[0u8; 32], b"x"), Err(ReportError::NoOperatorKey));
        // Duas denuncias iguais ficam diferentes (chave efemera).
        assert_ne!(seal_with(&op, b"x").unwrap(), seal_with(&op, b"x").unwrap());
    }

    #[test]
    fn chaves_do_operador_distintas() {
        let seed = [9u8; 32];
        assert_ne!(operator_report_public(&seed), operator_signing_public(&seed));
        assert_eq!(operator_signing_public(&seed), operator_signing_public(&seed));
    }
}
