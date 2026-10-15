//! Assinaturas Ed25519: so passam as que o verify_strict aceita.
//!
//! Com a chave publica "identidade" (um ponto de ordem pequena) e a assinatura
//! R = identidade, s = 0, a verificacao normal (verify) aceita QUALQUER
//! mensagem: e uma assinatura forjada que nao precisa de chave privada.

use ed25519_dalek::{Signature, VerifyingKey};
use qt_core::primitives::{verify_signature, SigningKeyPair};

fn identity_point() -> [u8; 32] {
    let mut p = [0u8; 32];
    p[0] = 1; // codificacao comprimida do ponto neutro
    p
}

#[test]
fn chave_fraca_com_assinatura_forjada_e_rejeitada() {
    let vk = VerifyingKey::from_bytes(&identity_point()).expect("o ponto decodifica");
    let mut forged = [0u8; 64];
    forged[..32].copy_from_slice(&identity_point()); // R = identidade, s = 0
    let sig = Signature::from_bytes(&forged);

    // A verificacao antiga deixava passar (fica aqui para mostrar o problema).
    use ed25519_dalek::Verifier;
    assert!(vk.verify(b"qualquer mensagem", &sig).is_ok());

    // A do projeto rejeita.
    assert!(verify_signature(&vk, b"qualquer mensagem", &sig).is_err());
}

#[test]
fn assinaturas_normais_continuam_a_passar() {
    let kp = SigningKeyPair::generate();
    for msg in [&b""[..], b"ola", &[7u8; 5000][..]] {
        let sig = kp.sign(msg);
        assert!(verify_signature(&kp.verifying_key, msg, &sig).is_ok());
        assert!(verify_signature(&kp.verifying_key, b"outra", &sig).is_err());
    }
}
