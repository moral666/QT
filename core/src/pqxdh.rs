//! PQXDH: a mesma logica de x3dh.rs, estendida com um KEM pos-quantico
//! (ML-KEM/Kyber, FIPS 203) combinado no HKDF final, para resistencia a um
//! adversario com computador quantico que capture o trafego hoje e tente
//! decifrar mais tarde ("harvest now, decrypt later").
//!
//! Atras da feature "pq" porque o crate `ml-kem` depende de `hybrid-array`,
//! que exige rustc >= 1.81 (o projeto suporta 1.75+ no core classico).
//! Ativa com:
//!   cargo build --features pq
//!   cargo test --features pq
//! Compilado e testado com sucesso com rustc 1.91.1 - ver
//! core/tests/pqxdh_flow.rs (teste completo Alice<->Bob passando).

use crate::primitives::{dh, hkdf_derive, verify_signature, DhKeyPair, SigningKeyPair};
use ed25519_dalek::{Signature, VerifyingKey};
use ml_kem::kem::{Decapsulate, Encapsulate};
use ml_kem::{Ciphertext, KemCore, MlKem768};
use rand_core::OsRng;
use thiserror::Error;
use x25519_dalek::PublicKey;

#[derive(Debug, Error)]
pub enum PqxdhError {
    #[error(transparent)]
    Crypto(#[from] crate::primitives::CryptoError),
}

/// Par de chaves ML-KEM-768 (Kyber) de longo prazo, publicado ao lado da
/// signed pre-key classica. Gerado uma vez, rotacionado no mesmo ciclo que
/// a signed pre-key (ex.: a cada 7 dias).
pub struct PqPreKeyPair {
    decapsulation_key: <MlKem768 as KemCore>::DecapsulationKey,
    pub encapsulation_key: <MlKem768 as KemCore>::EncapsulationKey,
}

impl PqPreKeyPair {
    pub fn generate() -> Self {
        let (decapsulation_key, encapsulation_key) = MlKem768::generate(&mut OsRng);
        Self { decapsulation_key, encapsulation_key }
    }
}

pub struct PqPreKeyBundle {
    pub identity_key: PublicKey,
    pub identity_signing_key: VerifyingKey,
    pub signed_pre_key: PublicKey,
    pub signed_pre_key_signature: Signature,
    pub pq_pre_key: <MlKem768 as KemCore>::EncapsulationKey,
    pub one_time_pre_key: Option<PublicKey>,
}

pub fn sign_pre_key(identity_signing: &SigningKeyPair, signed_pre_key: &PublicKey) -> Signature {
    identity_signing.sign(signed_pre_key.as_bytes())
}

pub struct PqxdhInitResult {
    pub shared_secret: [u8; 32],
    pub ephemeral_public: PublicKey,
    pub pq_ciphertext: Ciphertext<MlKem768>,
}

pub fn pqxdh_initiate(
    my_identity: &DhKeyPair,
    their_bundle: &PqPreKeyBundle,
) -> Result<PqxdhInitResult, PqxdhError> {
    verify_signature(
        &their_bundle.identity_signing_key,
        their_bundle.signed_pre_key.as_bytes(),
        &their_bundle.signed_pre_key_signature,
    )?;

    let ephemeral = DhKeyPair::generate();

    let dh1 = dh(&my_identity.private, &their_bundle.signed_pre_key);
    let dh2 = dh(&ephemeral.private, &their_bundle.identity_key);
    let dh3 = dh(&ephemeral.private, &their_bundle.signed_pre_key);
    let dh4 = their_bundle
        .one_time_pre_key
        .as_ref()
        .map(|opk| dh(&ephemeral.private, opk));

    let (pq_ciphertext, pq_shared_secret) =
        their_bundle.pq_pre_key.encapsulate(&mut OsRng).expect("encapsulamento ML-KEM falhou");

    let mut combined = Vec::with_capacity(32 * 5);
    combined.extend_from_slice(&dh1);
    combined.extend_from_slice(&dh2);
    combined.extend_from_slice(&dh3);
    if let Some(d4) = dh4 {
        combined.extend_from_slice(&d4);
    }
    combined.extend_from_slice(pq_shared_secret.as_slice());

    let derived = hkdf_derive(&combined, &[], b"PQXDH_v1", 32);
    let mut shared_secret = [0u8; 32];
    shared_secret.copy_from_slice(&derived);

    Ok(PqxdhInitResult { shared_secret, ephemeral_public: ephemeral.public, pq_ciphertext })
}

pub fn pqxdh_respond(
    my_identity: &DhKeyPair,
    my_signed_pre_key: &DhKeyPair,
    my_pq_pre_key: &PqPreKeyPair,
    my_one_time_pre_key: Option<&DhKeyPair>,
    their_identity_pub: &PublicKey,
    their_ephemeral_pub: &PublicKey,
    pq_ciphertext: &Ciphertext<MlKem768>,
) -> [u8; 32] {
    let dh1 = dh(&my_signed_pre_key.private, their_identity_pub);
    let dh2 = dh(&my_identity.private, their_ephemeral_pub);
    let dh3 = dh(&my_signed_pre_key.private, their_ephemeral_pub);
    let dh4 = my_one_time_pre_key.map(|opk| dh(&opk.private, their_ephemeral_pub));

    let pq_shared_secret = my_pq_pre_key
        .decapsulation_key
        .decapsulate(pq_ciphertext)
        .expect("decapsulamento ML-KEM falhou");

    let mut combined = Vec::with_capacity(32 * 5);
    combined.extend_from_slice(&dh1);
    combined.extend_from_slice(&dh2);
    combined.extend_from_slice(&dh3);
    if let Some(d4) = dh4 {
        combined.extend_from_slice(&d4);
    }
    combined.extend_from_slice(pq_shared_secret.as_slice());

    let derived = hkdf_derive(&combined, &[], b"PQXDH_v1", 32);
    let mut shared_secret = [0u8; 32];
    shared_secret.copy_from_slice(&derived);
    shared_secret
}
