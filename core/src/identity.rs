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

/// Bundle v3 (apps ate a 0.9.2): ainda aceite pelo relay, ja nao publicado.
pub const BUNDLE_VERSION: u8 = 3;
pub const PQ_PUBLIC_KEY_BYTES: usize = 1184;
/// [1 ver][32 identity][32 signing][32 signed_prekey][64 assinatura][4 pq_id BE][1184 pq_pub]
pub const BUNDLE_LEN: usize = 1 + 32 + 32 + 32 + 64 + 4 + PQ_PUBLIC_KEY_BYTES;

/// Bundle v4 (desde a app 0.9.3): a signed pre-key tem numero (para poder rodar) e o
/// bundle tem a hora em que foi criado, tudo coberto pela assinatura.
pub const BUNDLE_VERSION_V4: u8 = 4;
/// [1 ver=4][32 identity][32 signing][4 spk_id BE][32 signed_prekey][8 criado_em BE (s unix)]
/// [64 assinatura][4 pq_id BE][1184 pq_pub]
pub const BUNDLE_V4_LEN: usize = 1 + 32 + 32 + 4 + 32 + 8 + 64 + 4 + PQ_PUBLIC_KEY_BYTES;

const BUNDLE_SIG_CONTEXT_V2: &[u8] = b"QT-PQXDH-v2-bundle";
const BUNDLE_SIG_CONTEXT_V4: &[u8] = b"QT-PQXDH-v4-bundle";
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

/// Mensagem assinada no bundle v4: alem do v3, cobre o numero da signed pre-key, a
/// hora de criacao e o numero da prekey ML-KEM (um relay nao consegue servir uma
/// signed pre-key antiga como se fosse recente, nem trocar os numeros).
pub fn bundle_v4_signed_message(
    identity_key: &PublicKey,
    signed_pre_key_id: u32,
    signed_pre_key: &PublicKey,
    created_at: u64,
    pq_pre_key_id: u32,
    pq_pre_key_bytes: &[u8],
) -> Vec<u8> {
    let mut m = BUNDLE_SIG_CONTEXT_V4.to_vec();
    m.extend_from_slice(identity_key.as_bytes());
    m.extend_from_slice(&signed_pre_key_id.to_be_bytes());
    m.extend_from_slice(signed_pre_key.as_bytes());
    m.extend_from_slice(&created_at.to_be_bytes());
    m.extend_from_slice(&pq_pre_key_id.to_be_bytes());
    m.extend_from_slice(pq_pre_key_bytes);
    m
}

/// Serializa e ASSINA o bundle v4 a publicar.
pub fn encode_bundle_v4(
    identity_key: &PublicKey,
    signing: &SigningKeyPair,
    signed_pre_key_id: u32,
    signed_pre_key: &PublicKey,
    created_at: u64,
    pq_pre_key_id: u32,
    pq_pre_key_bytes: &[u8],
) -> Option<Vec<u8>> {
    if pq_pre_key_bytes.len() != PQ_PUBLIC_KEY_BYTES {
        return None;
    }
    let msg = bundle_v4_signed_message(identity_key, signed_pre_key_id, signed_pre_key, created_at, pq_pre_key_id, pq_pre_key_bytes);
    let sig = signing.sign(&msg);
    let mut out = Vec::with_capacity(BUNDLE_V4_LEN);
    out.push(BUNDLE_VERSION_V4);
    out.extend_from_slice(identity_key.as_bytes());
    out.extend_from_slice(&signing.verifying_key.to_bytes());
    out.extend_from_slice(&signed_pre_key_id.to_be_bytes());
    out.extend_from_slice(signed_pre_key.as_bytes());
    out.extend_from_slice(&created_at.to_be_bytes());
    out.extend_from_slice(&sig.to_bytes());
    out.extend_from_slice(&pq_pre_key_id.to_be_bytes());
    out.extend_from_slice(pq_pre_key_bytes);
    Some(out)
}

/// Campos de um bundle v4 ja separados (a assinatura ainda NAO esta verificada:
/// usar `verify_bundle_v4`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleV4 {
    pub identity: [u8; 32],
    pub signing: [u8; 32],
    pub signed_pre_key_id: u32,
    pub signed_pre_key: [u8; 32],
    pub created_at: u64,
    pub signature: [u8; 64],
    pub pq_pre_key_id: u32,
    pub pq_pre_key: Vec<u8>,
}

/// Separa os campos de um bundle v4 (so formato e tamanhos).
pub fn parse_bundle_v4(bytes: &[u8]) -> Result<BundleV4, IdentityError> {
    if bytes.len() != BUNDLE_V4_LEN || bytes[0] != BUNDLE_VERSION_V4 {
        return Err(IdentityError::BadFormat);
    }
    let f = |a: usize, b: usize| bytes.get(a..b).ok_or(IdentityError::BadFormat);
    let u32_at = |a: usize| -> Result<u32, IdentityError> {
        Ok(u32::from_be_bytes(f(a, a + 4)?.try_into().map_err(|_| IdentityError::BadFormat)?))
    };
    Ok(BundleV4 {
        identity: f(1, 33)?.try_into().map_err(|_| IdentityError::BadFormat)?,
        signing: f(33, 65)?.try_into().map_err(|_| IdentityError::BadFormat)?,
        signed_pre_key_id: u32_at(65)?,
        signed_pre_key: f(69, 101)?.try_into().map_err(|_| IdentityError::BadFormat)?,
        created_at: u64::from_be_bytes(f(101, 109)?.try_into().map_err(|_| IdentityError::BadFormat)?),
        signature: f(109, 173)?.try_into().map_err(|_| IdentityError::BadFormat)?,
        pq_pre_key_id: u32_at(173)?,
        pq_pre_key: f(177, BUNDLE_V4_LEN)?.to_vec(),
    })
}

/// Verifica a assinatura de um bundle v4 com a chave Ed25519 nele contida.
pub fn verify_bundle_v4(b: &BundleV4) -> Result<(), IdentityError> {
    let vk = VerifyingKey::from_bytes(&b.signing).map_err(|_| IdentityError::BadFormat)?;
    let msg = bundle_v4_signed_message(
        &PublicKey::from(b.identity),
        b.signed_pre_key_id,
        &PublicKey::from(b.signed_pre_key),
        b.created_at,
        b.pq_pre_key_id,
        &b.pq_pre_key,
    );
    verify_signature(&vk, &msg, &Signature::from_bytes(&b.signature)).map_err(|_| IdentityError::BadSignature)
}

/// Verifica formato, dono (chave Ed25519 == `expected_signing`) e assinatura.
/// Aceita o bundle v4 (atual) e o v3 (apps ate a 0.9.2).
pub fn verify_published_bundle(bytes: &[u8], expected_signing: &[u8; 32]) -> Result<(), IdentityError> {
    if bytes.first() == Some(&BUNDLE_VERSION_V4) {
        let b = parse_bundle_v4(bytes)?;
        if &b.signing != expected_signing {
            return Err(IdentityError::WrongOwner);
        }
        return verify_bundle_v4(&b);
    }
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

    #[test]
    fn bundle_v4_assinado_cobre_numeros_e_data() {
        let id = DhKeyPair::generate();
        let spk = DhKeyPair::generate();
        let sg = SigningKeyPair::generate();
        let pq = [9u8; PQ_PUBLIC_KEY_BYTES];
        let b = encode_bundle_v4(&id.public, &sg, 7, &spk.public, 1_790_000_000, 3, &pq).unwrap();
        assert_eq!(b.len(), BUNDLE_V4_LEN);
        let me = sg.verifying_key.to_bytes();
        assert_eq!(verify_published_bundle(&b, &me), Ok(()));
        let p = parse_bundle_v4(&b).unwrap();
        assert_eq!((p.signed_pre_key_id, p.created_at, p.pq_pre_key_id), (7, 1_790_000_000, 3));
        assert_eq!(p.identity, id.public.to_bytes());
        assert_eq!(p.signed_pre_key, spk.public.to_bytes());
        assert_eq!(p.pq_pre_key, pq.to_vec());
        let outro = SigningKeyPair::generate().verifying_key.to_bytes();
        assert_eq!(verify_published_bundle(&b, &outro), Err(IdentityError::WrongOwner));
        // Mudar o numero da signed pre-key, a data ou o numero da prekey PQ parte a assinatura.
        for pos in [66usize, 105, 176] {
            let mut t = b.clone();
            t[pos] ^= 1;
            assert_eq!(verify_published_bundle(&t, &me), Err(IdentityError::BadSignature), "byte {pos}");
        }
        assert_eq!(parse_bundle_v4(&b[..200]), Err(IdentityError::BadFormat));
    }
}
