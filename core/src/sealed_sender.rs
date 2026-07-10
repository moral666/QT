//! Sealed sender: esconde a identidade do REMETENTE do servidor/relay.
//! O servidor continua a saber o DESTINATARIO (precisa disso para rotear
//! a mensagem para a fila certa), mas nao consegue saber quem a enviou -
//! so o destinatario, que tem a chave privada de identidade correspondente,
//! consegue "abrir o envelope" e descobrir o remetente.
//!
//! Tecnica: cifra anonima (ECIES-like) contra a chave publica de
//! identidade do destinatario. O remetente gera um par de chaves EFEMERO
//! por mensagem, faz DH com a chave publica do destinatario, deriva uma
//! chave simetrica via HKDF, e cifra a sua propria chave de identidade
//! publica com ChaCha20-Poly1305. So quem tem a chave privada
//! correspondente ao destinatario consegue reproduzir o DH e decifrar.
//!
//! Isto e independente do Double Ratchet (`ratchet.rs`) - o servidor nunca
//! viu o conteudo da mensagem de qualquer forma; isto tapa o ultimo campo
//! que ainda estava em texto simples no protocolo do servidor (`from`).

use crate::primitives::{aead_decrypt, aead_encrypt, dh, hkdf_derive, CryptoError, DhKeyPair};
use x25519_dalek::PublicKey;

/// Cifra a identidade publica do remetente para que so o dono de
/// `recipient_identity_pub` a consiga ler. `sender_identity_pub` e tipicamente
/// a chave de identidade X25519 do remetente (a mesma usada no X3DH).
///
/// Formato do envelope: [32 bytes ephemeral_public][resto = AEAD(ciphertext)]
pub fn seal_sender_identity(
    sender_identity_pub: &PublicKey,
    recipient_identity_pub: &PublicKey,
) -> Vec<u8> {
    let ephemeral = DhKeyPair::generate();
    let shared_secret = dh(&ephemeral.private, recipient_identity_pub);
    let key_material = hkdf_derive(&shared_secret, &[], b"SealedSender_v1", 32);
    let mut key = [0u8; 32];
    key.copy_from_slice(&key_material);

    // O ephemeral_public serve de AAD - vincula o envelope a esta chave
    // efemera especifica, impedindo que um atacante troque o cabecalho.
    let ciphertext = aead_encrypt(&key, sender_identity_pub.as_bytes(), ephemeral.public.as_bytes());

    let mut out = Vec::with_capacity(32 + ciphertext.len());
    out.extend_from_slice(ephemeral.public.as_bytes());
    out.extend_from_slice(&ciphertext);
    out
}

/// Abre o envelope, devolvendo a chave publica de identidade do remetente.
/// So funciona com a chave privada de identidade correta do destinatario.
pub fn unseal_sender_identity(
    my_identity: &DhKeyPair,
    envelope: &[u8],
) -> Result<PublicKey, CryptoError> {
    if envelope.len() < 32 {
        return Err(CryptoError::InvalidLength);
    }
    let ephemeral_pub_bytes: [u8; 32] =
        envelope[0..32].try_into().map_err(|_| CryptoError::InvalidLength)?;
    let ephemeral_pub = PublicKey::from(ephemeral_pub_bytes);
    let ciphertext = &envelope[32..];

    let shared_secret = dh(&my_identity.private, &ephemeral_pub);
    let key_material = hkdf_derive(&shared_secret, &[], b"SealedSender_v1", 32);
    let mut key = [0u8; 32];
    key.copy_from_slice(&key_material);

    let plaintext = aead_decrypt(&key, ciphertext, &ephemeral_pub_bytes)?;
    let sender_bytes: [u8; 32] =
        plaintext.try_into().map_err(|_| CryptoError::InvalidLength)?;
    Ok(PublicKey::from(sender_bytes))
}
