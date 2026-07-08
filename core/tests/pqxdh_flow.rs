//! Teste do fluxo PQXDH (X25519 + ML-KEM). So compila/corre com
//! `cargo test --features pq` (ver Cargo.toml: required-features).
//! Nao foi possivel executar isto no ambiente onde o projeto foi gerado
//! (toolchain 1.75, ml-kem exige 1.81+) - correr localmente para validar.

use secure_messenger_core::pqxdh::{
    pqxdh_initiate, pqxdh_respond, sign_pre_key, PqPreKeyBundle, PqPreKeyPair,
};
use secure_messenger_core::primitives::{DhKeyPair, SigningKeyPair};
use secure_messenger_core::ratchet::RatchetState;

#[test]
fn fluxo_completo_alice_bob_pqxdh() {
    let alice_identity = DhKeyPair::generate();

    let bob_identity = DhKeyPair::generate();
    let bob_identity_signing = SigningKeyPair::generate();
    let bob_signed_pre_key = DhKeyPair::generate();
    let bob_pq_pre_key = PqPreKeyPair::generate();
    let bob_one_time_pre_key = DhKeyPair::generate();

    let signature = sign_pre_key(&bob_identity_signing, &bob_signed_pre_key.public);
    let bob_bundle = PqPreKeyBundle {
        identity_key: bob_identity.public,
        identity_signing_key: bob_identity_signing.verifying_key,
        signed_pre_key: bob_signed_pre_key.public,
        signed_pre_key_signature: signature,
        pq_pre_key: bob_pq_pre_key.encapsulation_key.clone(),
        one_time_pre_key: Some(bob_one_time_pre_key.public),
    };

    let init_result = pqxdh_initiate(&alice_identity, &bob_bundle)
        .expect("assinatura da signed pre-key deveria ser valida");

    let bob_shared_secret = pqxdh_respond(
        &bob_identity,
        &bob_signed_pre_key,
        &bob_pq_pre_key,
        Some(&bob_one_time_pre_key),
        &alice_identity.public,
        &init_result.ephemeral_public,
        &init_result.pq_ciphertext,
    );

    assert_eq!(
        init_result.shared_secret, bob_shared_secret,
        "PQXDH: segredos compartilhados devem coincidir em ambos os lados"
    );

    let mut alice_state =
        RatchetState::init_as_initiator(init_result.shared_secret, bob_signed_pre_key.public);
    let mut bob_state = RatchetState::init_as_responder(bob_shared_secret, bob_signed_pre_key);

    let enviado = alice_state.encrypt(b"mensagem pos-quantica").unwrap();
    let recebido = bob_state.decrypt(&enviado).unwrap();
    assert_eq!(recebido, b"mensagem pos-quantica");
}
