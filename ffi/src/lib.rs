//! Bindings FFI (uniffi) do nucleo criptografico. Desenho DELIBERADAMENTE
//! funcional (bytes dentro, bytes fora, sem objetos com estado mutavel
//! partilhado entre linguagens) - mais simples e mais robusto de gerar
//! bindings corretos em Kotlin/Swift/Python do que passar referencias vivas
//! de structs Rust complexas para o outro lado da fronteira FFI.
//!
//! O cliente movel (Android/iOS) e responsavel por guardar os bytes de
//! estado (identidade, sessao) no seu proprio armazenamento seguro -
//! exatamente o mesmo padrao que `storage/` ja usa no lado desktop/CLI.

use secure_messenger_core::primitives::{DhKeyPair, SigningKeyPair};
use secure_messenger_core::ratchet::{EncryptedMessage, RatchetState};
use secure_messenger_core::sealed_sender::{seal_sender_identity, unseal_sender_identity};
use secure_messenger_core::x3dh::{self, sign_pre_key, PreKeyBundle};
use x25519_dalek::{PublicKey, StaticSecret};

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("assinatura invalida - a signed pre-key pode ter sido adulterada")]
    InvalidSignature,
    #[error("falha ao cifrar ou decifrar: {reason}")]
    CryptoFailure { reason: String },
    #[error("bytes de entrada com tamanho invalido para: {field}")]
    InvalidLength { field: String },
}

fn to_array32(bytes: &[u8], field: &str) -> Result<[u8; 32], FfiError> {
    bytes
        .try_into()
        .map_err(|_| FfiError::InvalidLength { field: field.to_string() })
}

// ---------- Geração de chaves ----------

#[derive(uniffi::Record)]
pub struct KeyPairBytes {
    pub private: Vec<u8>,
    pub public: Vec<u8>,
}

#[uniffi::export]
pub fn generate_dh_keypair() -> KeyPairBytes {
    let kp = DhKeyPair::generate();
    KeyPairBytes { private: kp.private.to_bytes().to_vec(), public: kp.public.as_bytes().to_vec() }
}

#[uniffi::export]
pub fn generate_signing_keypair() -> KeyPairBytes {
    let kp = SigningKeyPair::generate();
    KeyPairBytes { private: kp.to_bytes().to_vec(), public: kp.verifying_key.to_bytes().to_vec() }
}

#[uniffi::export]
pub fn sign_signed_pre_key(
    signing_private: Vec<u8>,
    signed_pre_key_public: Vec<u8>,
) -> Result<Vec<u8>, FfiError> {
    let signing_key = SigningKeyPair::from_bytes(&to_array32(&signing_private, "signing_private")?);
    let spk_public = PublicKey::from(to_array32(&signed_pre_key_public, "signed_pre_key_public")?);
    let signature = sign_pre_key(&signing_key, &spk_public);
    Ok(signature.to_bytes().to_vec())
}

// ---------- X3DH ----------

#[derive(uniffi::Record)]
pub struct X3dhInitiateResult {
    pub shared_secret: Vec<u8>,
    pub ephemeral_public: Vec<u8>,
}

/// Executado por quem INICIA a conversa. `their_one_time_pre_key_public`
/// pode ser vazio (`None`) se o destinatario nao tinha nenhuma disponivel.
#[uniffi::export]
pub fn x3dh_initiate(
    my_identity_private: Vec<u8>,
    their_identity_public: Vec<u8>,
    their_identity_signing_public: Vec<u8>,
    their_signed_pre_key_public: Vec<u8>,
    their_signed_pre_key_signature: Vec<u8>,
    their_one_time_pre_key_public: Option<Vec<u8>>,
) -> Result<X3dhInitiateResult, FfiError> {
    let my_identity = DhKeyPair {
        private: StaticSecret::from(to_array32(&my_identity_private, "my_identity_private")?),
        public: PublicKey::from(&StaticSecret::from(to_array32(
            &my_identity_private,
            "my_identity_private",
        )?)),
    };

    let identity_signing_key =
        ed25519_dalek::VerifyingKey::from_bytes(&to_array32(
            &their_identity_signing_public,
            "their_identity_signing_public",
        )?)
        .map_err(|_| FfiError::InvalidLength { field: "their_identity_signing_public".into() })?;

    let signature_bytes = to_array32(&their_signed_pre_key_signature[0..32], "signature")
        .map(|_| ())
        .and_then(|_| {
            let arr: [u8; 64] = their_signed_pre_key_signature
                .as_slice()
                .try_into()
                .map_err(|_| FfiError::InvalidLength { field: "their_signed_pre_key_signature".into() })?;
            Ok(arr)
        })?;

    let bundle = PreKeyBundle {
        identity_key: PublicKey::from(to_array32(&their_identity_public, "their_identity_public")?),
        identity_signing_key: identity_signing_key,
        signed_pre_key: PublicKey::from(to_array32(
            &their_signed_pre_key_public,
            "their_signed_pre_key_public",
        )?),
        signed_pre_key_signature: ed25519_dalek::Signature::from_bytes(&signature_bytes),
        one_time_pre_key: their_one_time_pre_key_public
            .map(|b| to_array32(&b, "their_one_time_pre_key_public").map(PublicKey::from))
            .transpose()?,
    };

    let result = x3dh::x3dh_initiate(&my_identity, &bundle).map_err(|_| FfiError::InvalidSignature)?;

    Ok(X3dhInitiateResult {
        shared_secret: result.shared_secret.to_vec(),
        ephemeral_public: result.ephemeral_public.as_bytes().to_vec(),
    })
}

/// Executado por quem RESPONDE. `my_one_time_pre_key_private` deve ser
/// `None` exatamente quando o bundle publicado nao tinha nenhuma
/// disponivel (tem de corresponder ao que o iniciador usou).
#[uniffi::export]
pub fn x3dh_respond(
    my_identity_private: Vec<u8>,
    my_signed_pre_key_private: Vec<u8>,
    my_one_time_pre_key_private: Option<Vec<u8>>,
    their_identity_public: Vec<u8>,
    their_ephemeral_public: Vec<u8>,
) -> Result<Vec<u8>, FfiError> {
    let my_identity = DhKeyPair {
        private: StaticSecret::from(to_array32(&my_identity_private, "my_identity_private")?),
        public: PublicKey::from(&StaticSecret::from(to_array32(
            &my_identity_private,
            "my_identity_private",
        )?)),
    };
    let priv_bytes = to_array32(&my_signed_pre_key_private, "my_signed_pre_key_private")?;
    let my_signed_pre_key = DhKeyPair {
        private: StaticSecret::from(priv_bytes),
        public: PublicKey::from(&StaticSecret::from(priv_bytes)),
    };
    let my_one_time_pre_key = my_one_time_pre_key_private
        .map(|b| -> Result<DhKeyPair, FfiError> {
            let arr = to_array32(&b, "my_one_time_pre_key_private")?;
            Ok(DhKeyPair { private: StaticSecret::from(arr), public: PublicKey::from(&StaticSecret::from(arr)) })
        })
        .transpose()?;

    let their_identity = PublicKey::from(to_array32(&their_identity_public, "their_identity_public")?);
    let their_ephemeral =
        PublicKey::from(to_array32(&their_ephemeral_public, "their_ephemeral_public")?);

    let shared_secret = x3dh::x3dh_respond(
        &my_identity,
        &my_signed_pre_key,
        my_one_time_pre_key.as_ref(),
        &their_identity,
        &their_ephemeral,
    );

    Ok(shared_secret.to_vec())
}

// ---------- Double Ratchet ----------
// O estado do ratchet viaja sempre como bytes (RatchetState::to_bytes/
// from_bytes, ja existentes em core/ para a persistencia em storage/) -
// o cliente movel guarda-o exatamente como o CLI desktop guarda em SQLCipher.

#[uniffi::export]
pub fn ratchet_init_as_initiator(shared_secret: Vec<u8>, their_dh_public: Vec<u8>) -> Result<Vec<u8>, FfiError> {
    let secret = to_array32(&shared_secret, "shared_secret")?;
    let their_pub = PublicKey::from(to_array32(&their_dh_public, "their_dh_public")?);
    let state = RatchetState::init_as_initiator(secret, their_pub);
    Ok(state.to_bytes())
}

#[uniffi::export]
pub fn ratchet_init_as_responder(
    shared_secret: Vec<u8>,
    my_signed_pre_key_private: Vec<u8>,
) -> Result<Vec<u8>, FfiError> {
    let secret = to_array32(&shared_secret, "shared_secret")?;
    let priv_bytes = to_array32(&my_signed_pre_key_private, "my_signed_pre_key_private")?;
    let my_signed_pre_key = DhKeyPair {
        private: StaticSecret::from(priv_bytes),
        public: PublicKey::from(&StaticSecret::from(priv_bytes)),
    };
    let state = RatchetState::init_as_responder(secret, my_signed_pre_key);
    Ok(state.to_bytes())
}

#[derive(uniffi::Record)]
pub struct RatchetEncryptResult {
    pub new_state: Vec<u8>,
    pub dh_public: Vec<u8>,
    pub n: u32,
    pub ciphertext: Vec<u8>,
}

#[uniffi::export]
pub fn ratchet_encrypt(state_bytes: Vec<u8>, plaintext: Vec<u8>) -> Result<RatchetEncryptResult, FfiError> {
    let mut state = RatchetState::from_bytes(&state_bytes)
        .ok_or(FfiError::InvalidLength { field: "state_bytes".into() })?;
    let encrypted = state
        .encrypt(&plaintext)
        .map_err(|e| FfiError::CryptoFailure { reason: e.to_string() })?;
    Ok(RatchetEncryptResult {
        new_state: state.to_bytes(),
        dh_public: encrypted.dh_public.as_bytes().to_vec(),
        n: encrypted.n,
        ciphertext: encrypted.ciphertext,
    })
}

#[derive(uniffi::Record)]
pub struct RatchetDecryptResult {
    pub new_state: Vec<u8>,
    pub plaintext: Vec<u8>,
}

#[uniffi::export]
pub fn ratchet_decrypt(
    state_bytes: Vec<u8>,
    dh_public: Vec<u8>,
    n: u32,
    ciphertext: Vec<u8>,
) -> Result<RatchetDecryptResult, FfiError> {
    let mut state = RatchetState::from_bytes(&state_bytes)
        .ok_or(FfiError::InvalidLength { field: "state_bytes".into() })?;
    let msg = EncryptedMessage {
        dh_public: PublicKey::from(to_array32(&dh_public, "dh_public")?),
        n,
        ciphertext,
    };
    let plaintext = state
        .decrypt(&msg)
        .map_err(|e| FfiError::CryptoFailure { reason: e.to_string() })?;
    Ok(RatchetDecryptResult { new_state: state.to_bytes(), plaintext })
}

// ---------- Sealed sender ----------

#[uniffi::export]
pub fn seal_sender(sender_identity_public: Vec<u8>, recipient_identity_public: Vec<u8>) -> Result<Vec<u8>, FfiError> {
    let sender = PublicKey::from(to_array32(&sender_identity_public, "sender_identity_public")?);
    let recipient = PublicKey::from(to_array32(&recipient_identity_public, "recipient_identity_public")?);
    Ok(seal_sender_identity(&sender, &recipient))
}

#[uniffi::export]
pub fn unseal_sender(my_identity_private: Vec<u8>, envelope: Vec<u8>) -> Result<Vec<u8>, FfiError> {
    let priv_bytes = to_array32(&my_identity_private, "my_identity_private")?;
    let my_identity = DhKeyPair {
        private: StaticSecret::from(priv_bytes),
        public: PublicKey::from(&StaticSecret::from(priv_bytes)),
    };
    let sender_pub = unseal_sender_identity(&my_identity, &envelope)
        .map_err(|e| FfiError::CryptoFailure { reason: e.to_string() })?;
    Ok(sender_pub.as_bytes().to_vec())
}
