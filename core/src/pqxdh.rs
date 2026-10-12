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
use ml_kem::{Ciphertext, Encoded, EncodedSizeUser, KemCore, MlKem768};
use rand_core::OsRng;
use thiserror::Error;
use x25519_dalek::PublicKey;

#[derive(Debug, Error)]
pub enum PqxdhError {
    #[error(transparent)]
    Crypto(#[from] crate::primitives::CryptoError),
    #[error("chave publica ML-KEM invalida (tamanho ou codificacao nao canonica)")]
    InvalidPqPublicKey,
    #[error("chave privada ML-KEM invalida ou corrompida")]
    InvalidPqPrivateKey,
    #[error("ciphertext ML-KEM com tamanho invalido")]
    InvalidPqCiphertext,
    #[error("falha na operacao ML-KEM")]
    KemFailure,
}

/// Tamanho em bytes da chave de decapsulacao ML-KEM-768 serializada
/// (FIPS 203: dk_pke || ek || H(ek) || z).
pub const PQ_PRIVATE_KEY_LEN: usize = 2400;
/// Tamanho em bytes da chave de encapsulacao (publica) ML-KEM-768.
pub const PQ_PUBLIC_KEY_LEN: usize = 1184;
/// Tamanho em bytes do ciphertext ML-KEM-768.
pub const PQ_CIPHERTEXT_LEN: usize = 1088;
// Offset da chave de encapsulacao embebida dentro da chave de decapsulacao.
const EK_OFFSET_IN_DK: usize = 1152;

type PqEncapsulationKey = <MlKem768 as KemCore>::EncapsulationKey;
type PqDecapsulationKey = <MlKem768 as KemCore>::DecapsulationKey;

/// Desserializa e VALIDA uma chave publica ML-KEM-768: tamanho exato e
/// codificacao canonica (re-codificar tem de devolver os mesmos bytes, o que
/// rejeita coeficientes fora do intervalo - FIPS 203 §7.2).
pub fn pq_public_key_from_bytes(bytes: &[u8]) -> Result<PqEncapsulationKey, PqxdhError> {
    if bytes.len() != PQ_PUBLIC_KEY_LEN {
        return Err(PqxdhError::InvalidPqPublicKey);
    }
    let enc = Encoded::<PqEncapsulationKey>::try_from(bytes)
        .map_err(|_| PqxdhError::InvalidPqPublicKey)?;
    let key = PqEncapsulationKey::from_bytes(&enc);
    if key.as_bytes().as_slice() != bytes {
        return Err(PqxdhError::InvalidPqPublicKey);
    }
    Ok(key)
}

pub fn pq_public_key_to_bytes(key: &PqEncapsulationKey) -> Vec<u8> {
    key.as_bytes().as_slice().to_vec()
}

/// Valida o tamanho e desserializa um ciphertext ML-KEM-768.
pub fn pq_ciphertext_from_bytes(bytes: &[u8]) -> Result<Ciphertext<MlKem768>, PqxdhError> {
    if bytes.len() != PQ_CIPHERTEXT_LEN {
        return Err(PqxdhError::InvalidPqCiphertext);
    }
    Ciphertext::<MlKem768>::try_from(bytes).map_err(|_| PqxdhError::InvalidPqCiphertext)
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

    /// Serializa a chave PRIVADA (2400 bytes, formato FIPS 203 expandido - o
    /// `ml-kem 0.2.3` nao oferece API de seed). Material altamente sensivel:
    /// so deve ser entregue ao storage cifrado (SQLCipher), nunca a logs
    /// nem a Kotlin. A chave publica esta embebida e e recuperada em
    /// `from_bytes`.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.decapsulation_key.as_bytes().as_slice().to_vec()
    }

    /// Restaura o par a partir de `to_bytes`. Valida tamanho, canonicidade
    /// e coerencia interna (a chave publica embebida tem de re-codificar
    /// identica), para detetar estado corrompido em disco.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PqxdhError> {
        if bytes.len() != PQ_PRIVATE_KEY_LEN {
            return Err(PqxdhError::InvalidPqPrivateKey);
        }
        let enc = Encoded::<PqDecapsulationKey>::try_from(bytes)
            .map_err(|_| PqxdhError::InvalidPqPrivateKey)?;
        let decapsulation_key = PqDecapsulationKey::from_bytes(&enc);
        if decapsulation_key.as_bytes().as_slice() != bytes {
            return Err(PqxdhError::InvalidPqPrivateKey);
        }
        let ek_bytes = &bytes[EK_OFFSET_IN_DK..EK_OFFSET_IN_DK + PQ_PUBLIC_KEY_LEN];
        let encapsulation_key =
            pq_public_key_from_bytes(ek_bytes).map_err(|_| PqxdhError::InvalidPqPrivateKey)?;
        // Sanidade: encapsular com a publica e decapsular com a privada tem de coincidir.
        let (ct, ss1) = encapsulation_key
            .encapsulate(&mut OsRng)
            .map_err(|_| PqxdhError::InvalidPqPrivateKey)?;
        let ss2 = decapsulation_key
            .decapsulate(&ct)
            .map_err(|_| PqxdhError::InvalidPqPrivateKey)?;
        if ss1.as_slice() != ss2.as_slice() {
            return Err(PqxdhError::InvalidPqPrivateKey);
        }
        Ok(Self { decapsulation_key, encapsulation_key })
    }

    pub fn public_key_bytes(&self) -> Vec<u8> {
        pq_public_key_to_bytes(&self.encapsulation_key)
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

/// Contexto de dominio da assinatura do bundle PQXDH.
const BUNDLE_SIG_CONTEXT: &[u8] = b"QT-PQXDH-v1-bundle";

/// Mensagem assinada: `contexto || signed_pre_key || chave publica ML-KEM`.
/// A chave ML-KEM TEM de estar coberta pela assinatura: caso contrario, um
/// relay malicioso poderia substitui-la pela sua e anular a protecao
/// pos-quantica (o segredo ML-KEM passaria a ser conhecido do atacante).
fn bundle_signed_message(signed_pre_key: &PublicKey, pq_pre_key_bytes: &[u8]) -> Vec<u8> {
    let mut m = BUNDLE_SIG_CONTEXT.to_vec();
    m.extend_from_slice(signed_pre_key.as_bytes());
    m.extend_from_slice(pq_pre_key_bytes);
    m
}

/// Assina o bundle PQXDH (signed pre-key + prekey ML-KEM, 1184 bytes).
pub fn sign_pre_key(
    identity_signing: &SigningKeyPair,
    signed_pre_key: &PublicKey,
    pq_pre_key_bytes: &[u8],
) -> Signature {
    identity_signing.sign(&bundle_signed_message(signed_pre_key, pq_pre_key_bytes))
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
        &bundle_signed_message(
            &their_bundle.signed_pre_key,
            &pq_public_key_to_bytes(&their_bundle.pq_pre_key),
        ),
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
        their_bundle
            .pq_pre_key
            .encapsulate(&mut OsRng)
            .map_err(|_| PqxdhError::KemFailure)?;

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
) -> Result<[u8; 32], PqxdhError> {
    let dh1 = dh(&my_signed_pre_key.private, their_identity_pub);
    let dh2 = dh(&my_identity.private, their_ephemeral_pub);
    let dh3 = dh(&my_signed_pre_key.private, their_ephemeral_pub);
    let dh4 = my_one_time_pre_key.map(|opk| dh(&opk.private, their_ephemeral_pub));

    let pq_shared_secret = my_pq_pre_key
        .decapsulation_key
        .decapsulate(pq_ciphertext)
        .map_err(|_| PqxdhError::KemFailure)?;

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
    Ok(shared_secret)
}
