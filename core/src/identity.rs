//! Ligacao entre a identidade Ed25519 (assinar) e a X25519 (DH), formato
//! do bundle publicado no relay e SAFETY NUMBERS.
//!
//! - O bundle e assinado pela chave Ed25519 sobre
//!   `contexto || identity_x25519 || signed_pre_key || prekey ML-KEM`.
//!   Assim a chave Ed25519 ATESTA a chave X25519 de identidade: um relay
//!   nao pode juntar a identidade X25519 de outrem a uma assinatura valida.
//! - `verify_published_bundle` e usado pelo relay (sem a feature `pq`) para
//!   rejeitar bundles que nao pertencem a identidade autenticada.
//! - `safety_number` permite aos utilizadores comparar, fora de banda, que
//!   ambos veem as mesmas chaves (deteta MITM no QT Public ID/relay).

use crate::primitives::{verify_signature, SigningKeyPair};
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha512};
use thiserror::Error;
use x25519_dalek::PublicKey;

pub const BUNDLE_VERSION: u8 = 3;
pub const PQ_PUBLIC_KEY_BYTES: usize = 1184;
/// [1 ver][32 identity][32 signing][32 signed_prekey][64 assinatura][4 pq_id BE][1184 pq_pub]
pub const BUNDLE_LEN: usize = 1 + 32 + 32 + 32 + 64 + 4 + PQ_PUBLIC_KEY_BYTES;

const BUNDLE_SIG_CONTEXT_V2: &[u8] = b"QT-PQXDH-v2-bundle";
const AD_CONTEXT: &[u8] = b"QT-AD-v1";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("bundle com tamanho/versao invalidos")]
    BadFormat,
    #[error("o bundle nao pertence a identidade autenticada")]
    WrongOwner,
    #[error("assinatura do bundle invalida")]
    BadSignature,
}

/// Mensagem assinada no bundle (liga Ed25519 -> X25519 + prekeys).
pub fn bundle_signed_message(
    identity_key: &PublicKey,
    signed_pre_key: &PublicKey,
    pq_pre_key_bytes: &[u8],
) -> Vec<u8> {
    let mut m = BUNDLE_SIG_CONTEXT_V2.to_vec();
    m.extend_from_slice(identity_key.as_bytes());
    m.extend_from_slice(signed_pre_key.as_bytes());
    m.extend_from_slice(pq_pre_key_bytes);
    m
}

/// Serializa e ASSINA o bundle a publicar.
pub fn encode_bundle(
    identity_key: &PublicKey,
    signing: &SigningKeyPair,
    signed_pre_key: &PublicKey,
    pq_pre_key_id: u32,
    pq_pre_key_bytes: &[u8],
) -> Option<Vec<u8>> {
    if pq_pre_key_bytes.len() != PQ_PUBLIC_KEY_BYTES {
        return None;
    }
    let sig = signing.sign(&bundle_signed_message(identity_key, signed_pre_key, pq_pre_key_bytes));
    let mut out = Vec::with_capacity(BUNDLE_LEN);
    out.push(BUNDLE_VERSION);
    out.extend_from_slice(identity_key.as_bytes());
    out.extend_from_slice(&signing.verifying_key.to_bytes());
    out.extend_from_slice(signed_pre_key.as_bytes());
    out.extend_from_slice(&sig.to_bytes());
    out.extend_from_slice(&pq_pre_key_id.to_be_bytes());
    out.extend_from_slice(pq_pre_key_bytes);
    Some(out)
}

/// Verifica formato, dono (chave Ed25519 == `expected_signing`) e assinatura.
pub fn verify_published_bundle(bytes: &[u8], expected_signing: &[u8; 32]) -> Result<(), IdentityError> {
    if bytes.len() != BUNDLE_LEN || bytes[0] != BUNDLE_VERSION {
        return Err(IdentityError::BadFormat);
    }
    if &bytes[33..65] != expected_signing {
        return Err(IdentityError::WrongOwner);
    }
    let identity: [u8; 32] = bytes[1..33].try_into().map_err(|_| IdentityError::BadFormat)?;
    let spk: [u8; 32] = bytes[65..97].try_into().map_err(|_| IdentityError::BadFormat)?;
    let sig: [u8; 64] = bytes[97..161].try_into().map_err(|_| IdentityError::BadFormat)?;
    let vk = VerifyingKey::from_bytes(expected_signing).map_err(|_| IdentityError::BadFormat)?;
    let msg = bundle_signed_message(&PublicKey::from(identity), &PublicKey::from(spk), &bytes[165..]);
    verify_signature(&vk, &msg, &Signature::from_bytes(&sig)).map_err(|_| IdentityError::BadSignature)
}

/// Dados associados (AD) de uma sessao: identidades do iniciador e do
/// respondedor. Entram no AAD de cada mensagem do ratchet, por isso o
/// remetente e autenticado PELA SESSAO e nao por um campo do envelope.
pub fn session_associated_data(initiator_identity: &PublicKey, responder_identity: &PublicKey) -> Vec<u8> {
    let mut ad = AD_CONTEXT.to_vec();
    ad.extend_from_slice(initiator_identity.as_bytes());
    ad.extend_from_slice(responder_identity.as_bytes());
    ad
}

fn fingerprint_digits(identity: &[u8; 32], signing: &[u8; 32]) -> String {
    let mut h = Sha512::new();
    h.update(b"QT-SAFETY-v1");
    h.update(identity);
    h.update(signing);
    let mut digest = h.finalize_reset();
    for _ in 0..5200 {
        h.update(digest);
        h.update(identity);
        h.update(signing);
        digest = h.finalize_reset();
    }
    let mut s = String::with_capacity(30);
    for chunk in digest[..30].chunks(5) {
        let n = chunk.iter().fold(0u64, |a, b| (a << 8) | *b as u64);
        s.push_str(&format!("{:05}", n % 100_000));
    }
    s
}

/// Safety number de 60 digitos (12 grupos de 5), simetrico: ambos os lados
/// obtem o mesmo valor. Cobre as DUAS chaves de cada identidade.
pub fn safety_number(
    my_identity: &[u8; 32],
    my_signing: &[u8; 32],
    their_identity: &[u8; 32],
    their_signing: &[u8; 32],
) -> String {
    let a = fingerprint_digits(my_identity, my_signing);
    let b = fingerprint_digits(their_identity, their_signing);
    let (first, second) = if a <= b { (a, b) } else { (b, a) };
    let all = format!("{first}{second}");
    all.as_bytes()
        .chunks(5)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::DhKeyPair;

    fn keys() -> ([u8; 32], [u8; 32]) {
        (DhKeyPair::generate().public.to_bytes(), SigningKeyPair::generate().verifying_key.to_bytes())
    }

    #[test]
    fn safety_number_simetrico_e_sensivel_a_cada_chave() {
        let (ai, asg) = keys();
        let (bi, bsg) = keys();
        let n1 = safety_number(&ai, &asg, &bi, &bsg);
        assert_eq!(n1, safety_number(&bi, &bsg, &ai, &asg));
        assert_eq!(n1.replace(' ', "").len(), 60);
        assert_eq!(n1.split(' ').count(), 12);
        let (ci, csg) = keys();
        assert_ne!(n1, safety_number(&ai, &asg, &ci, &bsg), "trocar X25519 muda");
        assert_ne!(n1, safety_number(&ai, &asg, &bi, &csg), "trocar Ed25519 muda");
        let _ = ci;
    }

    #[test]
    fn bundle_assinado_valida_e_rejeita_dono_errado_e_adulteracao() {
        let id = DhKeyPair::generate();
        let spk = DhKeyPair::generate();
        let sg = SigningKeyPair::generate();
        let b = encode_bundle(&id.public, &sg, &spk.public, 1, &[7u8; PQ_PUBLIC_KEY_BYTES]).unwrap();
        let me = sg.verifying_key.to_bytes();
        assert_eq!(verify_published_bundle(&b, &me), Ok(()));
        let outro = SigningKeyPair::generate().verifying_key.to_bytes();
        assert_eq!(verify_published_bundle(&b, &outro), Err(IdentityError::WrongOwner));
        // trocar a identidade X25519 (mantendo assinatura) tem de falhar
        let mut t = b.clone();
        t[1..33].copy_from_slice(DhKeyPair::generate().public.as_bytes());
        assert_eq!(verify_published_bundle(&t, &me), Err(IdentityError::BadSignature));
        assert_eq!(verify_published_bundle(&b[..100], &me), Err(IdentityError::BadFormat));
    }
}
