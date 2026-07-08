//! X3DH (Extended Triple Diffie-Hellman) - estabelecimento do segredo inicial.
//!
//! Versao classica (sem componente pos-quantico - ver pqxdh.rs para a
//! variante PQXDH, que combina isto com ML-KEM/Kyber, atras da feature "pq").
//!
//! Ja cobre os dois TODOs criticos da v0.1:
//!   1. Assinatura Ed25519 da signed pre-key, verificada pelo iniciador -
//!      protege contra um servidor malicioso substituir a pre-key publicada.
//!   2. One-time pre-key (DH4), quando disponivel - fortalece a garantia de
//!      forward secrecy logo na primeira mensagem.

use crate::primitives::{dh, hkdf_derive, verify_signature, DhKeyPair, SigningKeyPair};
use ed25519_dalek::{Signature, VerifyingKey};
use thiserror::Error;
use x25519_dalek::PublicKey;

#[derive(Debug, Error)]
pub enum X3dhError {
    #[error(transparent)]
    Crypto(#[from] crate::primitives::CryptoError),
}

/// Bundle publico que um utilizador publica no servidor para permitir que
/// outros iniciem uma sessao com ele, mesmo estando offline.
pub struct PreKeyBundle {
    /// Chave de identidade X25519 (usada nos calculos DH).
    pub identity_key: PublicKey,
    /// Chave de assinatura Ed25519 (mesma identidade, uso separado: assinar).
    pub identity_signing_key: VerifyingKey,
    pub signed_pre_key: PublicKey,
    pub signed_pre_key_signature: Signature,
    /// Pre-key descartavel, consumida pelo servidor a cada novo handshake.
    /// None quando o servidor ficou sem one-time pre-keys disponiveis para
    /// este utilizador (X3DH ainda funciona, com garantia ligeiramente
    /// mais fraca na primeira mensagem - ver docs/protocol-spec.md).
    pub one_time_pre_key: Option<PublicKey>,
}

/// Gera a assinatura a publicar junto de uma signed pre-key nova. Chamado
/// pelo dono da identidade sempre que rotaciona a signed pre-key.
pub fn sign_pre_key(identity_signing: &SigningKeyPair, signed_pre_key: &PublicKey) -> Signature {
    identity_signing.sign(signed_pre_key.as_bytes())
}

/// Resultado do X3DH: o segredo compartilhado + a chave efemera publica que
/// o iniciador (Alice) deve enviar ao destinatario (Bob) junto da primeira mensagem.
pub struct X3dhInitResult {
    pub shared_secret: [u8; 32],
    pub ephemeral_public: PublicKey,
}

/// Executado por quem INICIA a conversa (ex.: Alice), usando o bundle
/// publico que Bob publicou no servidor.
pub fn x3dh_initiate(
    my_identity: &DhKeyPair,
    their_bundle: &PreKeyBundle,
) -> Result<X3dhInitResult, X3dhError> {
    // Verificar a assinatura da signed pre-key ANTES de confiar nela.
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

    let mut combined = Vec::with_capacity(32 * 4);
    combined.extend_from_slice(&dh1);
    combined.extend_from_slice(&dh2);
    combined.extend_from_slice(&dh3);
    if let Some(d4) = dh4 {
        combined.extend_from_slice(&d4);
    }

    let derived = hkdf_derive(&combined, &[], b"X3DH_v1", 32);
    let mut shared_secret = [0u8; 32];
    shared_secret.copy_from_slice(&derived);

    Ok(X3dhInitResult { shared_secret, ephemeral_public: ephemeral.public })
}

/// Executado por quem RESPONDE (ex.: Bob), ao receber a primeira mensagem
/// de Alice contendo a chave efemera publica dela.
///
/// `my_one_time_pre_key`: a private key da one-time pre-key entregue pelo
/// servidor a Alice (None se nao havia nenhuma disponivel - deve corresponder
/// exatamente ao que Alice recebeu, ou o DH4 nao bate certo).
pub fn x3dh_respond(
    my_identity: &DhKeyPair,
    my_signed_pre_key: &DhKeyPair,
    my_one_time_pre_key: Option<&DhKeyPair>,
    their_identity_pub: &PublicKey,
    their_ephemeral_pub: &PublicKey,
) -> [u8; 32] {
    let dh1 = dh(&my_signed_pre_key.private, their_identity_pub);
    let dh2 = dh(&my_identity.private, their_ephemeral_pub);
    let dh3 = dh(&my_signed_pre_key.private, their_ephemeral_pub);
    let dh4 = my_one_time_pre_key.map(|opk| dh(&opk.private, their_ephemeral_pub));

    let mut combined = Vec::with_capacity(32 * 4);
    combined.extend_from_slice(&dh1);
    combined.extend_from_slice(&dh2);
    combined.extend_from_slice(&dh3);
    if let Some(d4) = dh4 {
        combined.extend_from_slice(&d4);
    }

    let derived = hkdf_derive(&combined, &[], b"X3DH_v1", 32);
    let mut shared_secret = [0u8; 32];
    shared_secret.copy_from_slice(&derived);
    shared_secret
}
