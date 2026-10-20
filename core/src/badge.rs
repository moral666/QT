//! Badges (Founder, Co-founder, Dev, Tester, Contributor): certificados assinados pelo
//! emissor (o Nuno), sem nada guardado no servidor.
//!
//!   * O emissor tem uma chave Ed25519 so dele (fora do servidor e do git). A chave
//!     PUBLICA esta fixa em `ISSUER_PUBLIC_KEY` e e igual no Android e no iPhone.
//!   * Um certificado diz "a identidade X tem o badge B desde T" e so vale para essa
//!     identidade (a chave X25519, que e o contact_id). Copiar o certificado de outra
//!     pessoa nao serve: quem o recebe confere que veio da propria identidade.
//!   * Retirar um badge e um certificado novo com `BADGE_NONE` e data mais recente.
//!
//! FORMATO (106 bytes): `[1 versao=1][1 badge][8 emitido_em ms BE][32 identidade X25519]
//! [64 assinatura Ed25519]`. A assinatura cobre `DOMAIN` seguido dos primeiros 42 bytes.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

/// Chave publica do emissor (Ed25519, 32 bytes). Gerada pelo Nuno com
/// `cargo run -p qt_server --example badge -- chave`. Com zeros nenhum badge e aceite.
pub const ISSUER_PUBLIC_KEY: [u8; 32] = [0u8; 32];

pub const VERSION: u8 = 1;
pub const CERT_LEN: usize = 1 + 1 + 8 + 32 + 64;
const SIGNED_LEN: usize = CERT_LEN - 64;
const DOMAIN: &[u8] = b"SEALER-BADGE-v1";

pub const BADGE_NONE: u8 = 0;
pub const BADGE_FOUNDER: u8 = 1;
pub const BADGE_COFOUNDER: u8 = 2;
pub const BADGE_DEV: u8 = 3;
pub const BADGE_TESTER: u8 = 4;
pub const BADGE_CONTRIBUTOR: u8 = 5;
/// Maior numero de badge conhecido (os outros sao recusados).
pub const BADGE_MAX: u8 = BADGE_CONTRIBUTOR;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadgeCert {
    pub badge: u8,
    pub issued_at: i64,
    pub subject: [u8; 32],
}

fn signed_part(badge: u8, issued_at: i64, subject: &[u8; 32]) -> [u8; SIGNED_LEN] {
    let mut b = [0u8; SIGNED_LEN];
    b[0] = VERSION;
    b[1] = badge;
    b[2..10].copy_from_slice(&issued_at.to_be_bytes());
    b[10..42].copy_from_slice(subject);
    b
}

fn message(signed: &[u8]) -> Vec<u8> {
    [DOMAIN, signed].concat()
}

/// So o emissor usa isto (ferramenta `badge`). `seed` e a chave privada Ed25519 (32 bytes).
pub fn issue(seed: &[u8; 32], subject: &[u8; 32], badge: u8, issued_at: i64) -> Vec<u8> {
    let sk = SigningKey::from_bytes(seed);
    let signed = signed_part(badge, issued_at, subject);
    let sig = sk.sign(&message(&signed));
    [signed.as_slice(), &sig.to_bytes()].concat()
}

/// Chave publica de uma chave privada do emissor.
pub fn issuer_public(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

/// Confere formato, badge conhecido e assinatura com a chave `issuer`.
pub fn verify_with(issuer: &[u8; 32], cert: &[u8]) -> Option<BadgeCert> {
    if cert.len() != CERT_LEN || cert[0] != VERSION || cert[1] > BADGE_MAX {
        return None;
    }
    if issuer == &[0u8; 32] {
        return None;
    }
    let vk = VerifyingKey::from_bytes(issuer).ok()?;
    let sig = Signature::from_slice(&cert[SIGNED_LEN..]).ok()?;
    // `verify_strict`: recusa chaves fracas e assinaturas maleaveis.
    vk.verify_strict(&message(&cert[..SIGNED_LEN]), &sig).ok()?;
    Some(BadgeCert {
        badge: cert[1],
        issued_at: i64::from_be_bytes(cert[2..10].try_into().ok()?),
        subject: cert[10..42].try_into().ok()?,
    })
}

/// Confere com a chave fixa do emissor.
pub fn verify(cert: &[u8]) -> Option<BadgeCert> {
    verify_with(&ISSUER_PUBLIC_KEY, cert)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emitir_e_verificar() {
        let seed = [7u8; 32];
        let issuer = issuer_public(&seed);
        let who = [9u8; 32];
        let c = issue(&seed, &who, BADGE_TESTER, 1_700_000_000_000);
        assert_eq!(c.len(), CERT_LEN);
        let v = verify_with(&issuer, &c).unwrap();
        assert_eq!(
            (v.badge, v.issued_at, v.subject),
            (BADGE_TESTER, 1_700_000_000_000, who)
        );
        // Outra chave do emissor, bytes mexidos ou badge desconhecido: recusado.
        assert!(verify_with(&issuer_public(&[8u8; 32]), &c).is_none());
        let mut m = c.clone();
        m[1] = BADGE_FOUNDER;
        assert!(verify_with(&issuer, &m).is_none());
        let mut m = c.clone();
        m[20] ^= 1;
        assert!(verify_with(&issuer, &m).is_none());
        assert!(verify_with(&issuer, &issue(&seed, &who, BADGE_MAX + 1, 0)).is_none());
        assert!(verify_with(&issuer, &c[..CERT_LEN - 1]).is_none());
        // Com a chave por preencher (zeros) nada e aceite.
        if ISSUER_PUBLIC_KEY == [0u8; 32] {
            assert!(verify(&c).is_none());
        }
    }
}
