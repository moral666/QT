//! Primitivas criptograficas de baixo nivel.
//!
//! Toda a criptografia "de verdade" fica isolada aqui. O resto do core
//! (x3dh.rs, ratchet.rs) so deve chamar estas funcoes, nunca implementar
//! operacoes criptograficas diretamente. Isto facilita auditoria: um revisor
//! externo foca-se sobretudo neste ficheiro e no ratchet.rs.

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Key, Nonce,
};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use thiserror::Error;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroize;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("falha na decifragem: dados corrompidos ou chave errada")]
    DecryptionFailed,
    #[error("tamanho de nonce/ciphertext invalido")]
    InvalidLength,
    #[error("assinatura invalida: a pre-key pode ter sido adulterada/substituida")]
    SignatureInvalid,
}

/// Par de chaves Ed25519 usado para ASSINAR (nao para DH). E a chave de
/// identidade "de longo prazo" que assina a signed pre-key publicada no
/// servidor, permitindo a quem inicia um handshake verificar que a pre-key
/// realmente pertence a quem diz pertencer (protege contra um servidor
/// malicioso substituir a pre-key por uma sua).
pub struct SigningKeyPair {
    signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
}

impl SigningKeyPair {
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        Self { signing_key, verifying_key }
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }
}

/// Verifica uma assinatura Ed25519. Deve ser chamado SEMPRE antes de usar
/// uma signed pre-key recebida do servidor.
pub fn verify_signature(
    verifying_key: &VerifyingKey,
    message: &[u8],
    signature: &Signature,
) -> Result<(), CryptoError> {
    verifying_key
        .verify(message, signature)
        .map_err(|_| CryptoError::SignatureInvalid)
}

/// Par de chaves X25519 para operacoes Diffie-Hellman.
pub struct DhKeyPair {
    pub private: StaticSecret,
    pub public: PublicKey,
}

impl DhKeyPair {
    pub fn generate() -> Self {
        let private = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&private);
        Self { private, public }
    }
}

/// Diffie-Hellman puro sobre X25519.
pub fn dh(private: &StaticSecret, public: &PublicKey) -> [u8; 32] {
    private.diffie_hellman(public).to_bytes()
}

/// HKDF-SHA256 generico: deriva `length` bytes a partir de `ikm`, com `salt` e `info`.
pub fn hkdf_derive(ikm: &[u8], salt: &[u8], info: &[u8], length: usize) -> Vec<u8> {
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
    let mut okm = vec![0u8; length];
    hk.expand(info, &mut okm)
        .expect("comprimento de saida HKDF invalido");
    okm
}

/// Deriva (novo root key, nova chain key) a partir do root key atual e de uma saida DH.
/// Usado no passo de "DH ratchet" do Double Ratchet.
pub fn kdf_root_key(root_key: &[u8; 32], dh_output: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let okm = hkdf_derive(dh_output, root_key, b"DoubleRatchet_RootKDF_v1", 64);
    let mut new_root = [0u8; 32];
    let mut chain_key = [0u8; 32];
    new_root.copy_from_slice(&okm[0..32]);
    chain_key.copy_from_slice(&okm[32..64]);
    (new_root, chain_key)
}

/// Deriva (message key, proxima chain key) a partir da chain key atual.
/// Usado a cada mensagem enviada/recebida ("symmetric-key ratchet").
pub fn kdf_chain_key(chain_key: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let message_key = hkdf_derive(chain_key, &[], b"DoubleRatchet_MsgKey_v1", 32);
    let next_chain_key = hkdf_derive(chain_key, &[], b"DoubleRatchet_ChainKey_v1", 32);
    let mut mk = [0u8; 32];
    let mut nck = [0u8; 32];
    mk.copy_from_slice(&message_key);
    nck.copy_from_slice(&next_chain_key);
    (mk, nck)
}

/// Cifra com ChaCha20-Poly1305 (AEAD). Nonce aleatorio, prefixado ao ciphertext.
/// `aad` (associated data) deve incluir o header da mensagem (chave DH publica +
/// numero da mensagem) para vincular criptograficamente o header ao conteudo,
/// prevenindo adulteracao do header por um atacante na rede.
pub fn aead_encrypt(key: &[u8; 32], plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, Payload { msg: plaintext, aad })
        .expect("falha na cifragem AEAD (nao deveria acontecer com chave valida)");

    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    out
}

pub fn aead_decrypt(key: &[u8; 32], blob: &[u8], aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if blob.len() < 12 {
        return Err(CryptoError::InvalidLength);
    }
    let (nonce_bytes, ciphertext) = blob.split_at(12);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, Payload { msg: ciphertext, aad })
        .map_err(|_| CryptoError::DecryptionFailed)
}

/// Wrapper que zera a memoria ao sair de escopo - importante para chaves de
/// mensagem individuais, que devem ser destruidas imediatamente apos uso
/// (garante forward secrecy real, nao so "teorica").
#[derive(Zeroize)]
#[zeroize(drop)]
pub struct EphemeralKey(pub [u8; 32]);
