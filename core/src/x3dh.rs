//! X3DH (Extended Triple Diffie-Hellman) - estabelecimento do segredo inicial.
//!
//! NOTA IMPORTANTE PARA PRODUCAO: esta e uma implementacao didatica do X3DH
//! classico (3x DH). Antes de qualquer lancamento real, isto deve ser
//! estendido para PQXDH (adicionando um KEM pos-quantico como ML-KEM/Kyber
//! ao lado do X25519), como fez o Signal, para resistencia a "harvest now,
//! decrypt later". Ver docs/protocol-spec.md, secao "Pos-Quantico".
//!
//! Tambem em falta aqui, de forma deliberada para manter o exemplo legivel:
//! - Assinatura Ed25519 da signed pre-key (deve ser verificada antes de usar)
//! - One-time pre-keys (aqui usamos so identity key + signed pre-key)
//! Ambas devem ser adicionadas antes de qualquer uso em producao.

use crate::primitives::{dh, hkdf_derive, DhKeyPair};
use x25519_dalek::PublicKey;

/// Bundle publico que um utilizador publica no servidor para permitir que
/// outros iniciem uma sessao com ele, mesmo estando offline.
pub struct PreKeyBundle {
    pub identity_key: PublicKey,
    pub signed_pre_key: PublicKey,
    // TODO(producao): pub signed_pre_key_signature: ed25519_dalek::Signature,
    // TODO(producao): pub one_time_pre_key: Option<PublicKey>,
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
) -> X3dhInitResult {
    // TODO(producao): verificar their_bundle.signed_pre_key_signature contra
    // their_bundle.identity_key ANTES de prosseguir. Sem isso, um servidor
    // malicioso poderia substituir a signed pre-key (ataque de personificacao).

    let ephemeral = DhKeyPair::generate();

    let dh1 = dh(&my_identity.private, &their_bundle.signed_pre_key);
    let dh2 = dh(&ephemeral.private, &their_bundle.identity_key);
    let dh3 = dh(&ephemeral.private, &their_bundle.signed_pre_key);

    let mut combined = Vec::with_capacity(96);
    combined.extend_from_slice(&dh1);
    combined.extend_from_slice(&dh2);
    combined.extend_from_slice(&dh3);

    let derived = hkdf_derive(&combined, &[], b"X3DH_v1", 32);
    let mut shared_secret = [0u8; 32];
    shared_secret.copy_from_slice(&derived);

    X3dhInitResult {
        shared_secret,
        ephemeral_public: ephemeral.public,
    }
}

/// Executado por quem RESPONDE (ex.: Bob), ao receber a primeira mensagem
/// de Alice contendo a chave efemera publica dela.
pub fn x3dh_respond(
    my_identity: &DhKeyPair,
    my_signed_pre_key: &DhKeyPair,
    their_identity_pub: &PublicKey,
    their_ephemeral_pub: &PublicKey,
) -> [u8; 32] {
    let dh1 = dh(&my_signed_pre_key.private, their_identity_pub);
    let dh2 = dh(&my_identity.private, their_ephemeral_pub);
    let dh3 = dh(&my_signed_pre_key.private, their_ephemeral_pub);

    let mut combined = Vec::with_capacity(96);
    combined.extend_from_slice(&dh1);
    combined.extend_from_slice(&dh2);
    combined.extend_from_slice(&dh3);

    let derived = hkdf_derive(&combined, &[], b"X3DH_v1", 32);
    let mut shared_secret = [0u8; 32];
    shared_secret.copy_from_slice(&derived);
    shared_secret
}
