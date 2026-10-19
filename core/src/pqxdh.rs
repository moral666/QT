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

use crate::identity::{bundle_signed_message, verify_bundle_v4, BundleV4};
use crate::primitives::{dh, hkdf_derive, verify_signature, DhKeyPair, SigningKeyPair};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;
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

/// Assina o bundle PQXDH: liga a chave Ed25519 a identidade X25519, a signed
/// pre-key e a prekey ML-KEM (ver `identity::bundle_signed_message`).
pub fn sign_pre_key(
    identity_signing: &SigningKeyPair,
    identity_key: &PublicKey,
    signed_pre_key: &PublicKey,
    pq_pre_key_bytes: &[u8],
) -> Signature {
    identity_signing.sign(&bundle_signed_message(identity_key, signed_pre_key, pq_pre_key_bytes))
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
            &their_bundle.identity_key,
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

// ------------------------------------------------------------------ PQXDH v2
//
// Desde a app 0.9.3. Igual ao v1 nas trocas de chaves (DH1..DH4 + ML-KEM-768), mas a
// derivacao do segredo segue a especificacao PQXDH do Signal e acrescenta a
// "transcricao" do aperto de mao:
//
//   IKM  = 0xFF x 32 || DH1 || DH2 || DH3 [|| DH4] || ss_ML-KEM
//   salt = 0x00 x 32
//   info = "QT-PQXDH-v2" || SHA-256("QT-PQXDH-v2-transcript" || IK_A || IK_B || SPK_B
//                                   || EK_A || (OPK_B ou 32 zeros) || PQPK_B || ct)
//
// O segredo fica assim ligado a todas as chaves publicas e ao ciphertext ML-KEM
// usados: trocar qualquer um (por exemplo, um relay a servir outra prekey) da um
// segredo diferente dos dois lados. O v1 continua acima so para sessoes antigas.

const KDF_V2_INFO: &[u8] = b"QT-PQXDH-v2";
const TRANSCRIPT_V2: &[u8] = b"QT-PQXDH-v2-transcript";

#[allow(clippy::too_many_arguments)]
fn transcript_v2(
    ik_a: &PublicKey,
    ik_b: &PublicKey,
    spk_b: &PublicKey,
    ek_a: &PublicKey,
    opk_b: Option<&PublicKey>,
    pq_pk_b: &[u8],
    ct: &[u8],
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(TRANSCRIPT_V2);
    h.update(ik_a.as_bytes());
    h.update(ik_b.as_bytes());
    h.update(spk_b.as_bytes());
    h.update(ek_a.as_bytes());
    h.update(opk_b.map(|k| k.to_bytes()).unwrap_or([0u8; 32]));
    h.update(pq_pk_b);
    h.update(ct);
    h.finalize().into()
}

fn kdf_v2(dhs: &[[u8; 32]], pq_shared_secret: &[u8], transcript: &[u8; 32]) -> Result<[u8; 32], PqxdhError> {
    let mut ikm = Vec::with_capacity(32 * 6);
    ikm.extend_from_slice(&[0xFFu8; 32]);
    for d in dhs {
        ikm.extend_from_slice(d);
    }
    ikm.extend_from_slice(pq_shared_secret);
    let mut info = KDF_V2_INFO.to_vec();
    info.extend_from_slice(transcript);
    let mut derived = hkdf_derive(&ikm, &[0u8; 32], &info, 32);
    ikm.zeroize();
    let out: [u8; 32] = derived.as_slice().try_into().map_err(|_| PqxdhError::KemFailure)?;
    derived.zeroize();
    Ok(out)
}

pub struct PqxdhV2Init {
    pub shared_secret: [u8; 32],
    pub ephemeral_public: PublicKey,
    pub pq_ciphertext: Ciphertext<MlKem768>,
    pub signed_pre_key: PublicKey,
    pub signed_pre_key_id: u32,
    pub pq_pre_key_id: u32,
}

/// Inicia uma sessao PQXDH v2 com um bundle v4. Verifica primeiro a assinatura do
/// bundle (com a chave Ed25519 nele contida; o chamador confirma que essa chave e a
/// do contacto, pelo QT Public ID).
pub fn pqxdh_initiate_v2(
    my_identity: &DhKeyPair,
    bundle: &BundleV4,
    one_time_pre_key: Option<&PublicKey>,
) -> Result<PqxdhV2Init, PqxdhError> {
    verify_bundle_v4(bundle).map_err(|_| crate::primitives::CryptoError::SignatureInvalid)?;
    let pq_pre_key = pq_public_key_from_bytes(&bundle.pq_pre_key)?;
    let their_identity = PublicKey::from(bundle.identity);
    let spk = PublicKey::from(bundle.signed_pre_key);
    let ephemeral = DhKeyPair::generate();

    let mut dhs = vec![
        dh(&my_identity.private, &spk),
        dh(&ephemeral.private, &their_identity),
        dh(&ephemeral.private, &spk),
    ];
    if let Some(opk) = one_time_pre_key {
        dhs.push(dh(&ephemeral.private, opk));
    }
    let (pq_ciphertext, pq_shared_secret) =
        pq_pre_key.encapsulate(&mut OsRng).map_err(|_| PqxdhError::KemFailure)?;
    let t = transcript_v2(
        &my_identity.public,
        &their_identity,
        &spk,
        &ephemeral.public,
        one_time_pre_key,
        &bundle.pq_pre_key,
        pq_ciphertext.as_slice(),
    );
    let shared_secret = kdf_v2(&dhs, pq_shared_secret.as_slice(), &t);
    dhs.iter_mut().for_each(|d| d.zeroize());
    Ok(PqxdhV2Init {
        shared_secret: shared_secret?,
        ephemeral_public: ephemeral.public,
        pq_ciphertext,
        signed_pre_key: spk,
        signed_pre_key_id: bundle.signed_pre_key_id,
        pq_pre_key_id: bundle.pq_pre_key_id,
    })
}

/// Lado de quem recebe a primeira mensagem (PQXDH v2).
pub fn pqxdh_respond_v2(
    my_identity: &DhKeyPair,
    my_signed_pre_key: &DhKeyPair,
    my_pq_pre_key: &PqPreKeyPair,
    my_one_time_pre_key: Option<&DhKeyPair>,
    their_identity_pub: &PublicKey,
    their_ephemeral_pub: &PublicKey,
    pq_ciphertext: &Ciphertext<MlKem768>,
) -> Result<[u8; 32], PqxdhError> {
    let mut dhs = vec![
        dh(&my_signed_pre_key.private, their_identity_pub),
        dh(&my_identity.private, their_ephemeral_pub),
        dh(&my_signed_pre_key.private, their_ephemeral_pub),
    ];
    if let Some(opk) = my_one_time_pre_key {
        dhs.push(dh(&opk.private, their_ephemeral_pub));
    }
    let pq_shared_secret = my_pq_pre_key
        .decapsulation_key
        .decapsulate(pq_ciphertext)
        .map_err(|_| PqxdhError::KemFailure)?;
    let t = transcript_v2(
        their_identity_pub,
        &my_identity.public,
        &my_signed_pre_key.public,
        their_ephemeral_pub,
        my_one_time_pre_key.map(|k| &k.public),
        &my_pq_pre_key.public_key_bytes(),
        pq_ciphertext.as_slice(),
    );
    let out = kdf_v2(&dhs, pq_shared_secret.as_slice(), &t);
    dhs.iter_mut().for_each(|d| d.zeroize());
    out
}
