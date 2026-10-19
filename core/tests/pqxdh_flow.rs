//! Teste do fluxo PQXDH (X25519 + ML-KEM). So compila/corre com
//! `cargo test --features pq` (ver Cargo.toml: required-features).

use qt_core::pqxdh::{
    pqxdh_initiate, pqxdh_respond, sign_pre_key, PqPreKeyBundle, PqPreKeyPair,
};
use qt_core::primitives::{DhKeyPair, SigningKeyPair};
use qt_core::ratchet::RatchetState;

#[test]
fn fluxo_completo_alice_bob_pqxdh() {
    let alice_identity = DhKeyPair::generate();

    let bob_identity = DhKeyPair::generate();
    let bob_identity_signing = SigningKeyPair::generate();
    let bob_signed_pre_key = DhKeyPair::generate();
    let bob_pq_pre_key = PqPreKeyPair::generate();
    let bob_one_time_pre_key = DhKeyPair::generate();

    let signature = sign_pre_key(&bob_identity_signing, &bob_identity.public, &bob_signed_pre_key.public, &bob_pq_pre_key.public_key_bytes());
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
    )
    .expect("respond");

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

// ------------------------------------------------------------- PQXDH v2 (bundle v4)

fn bob_v4(signed_pre_key: &DhKeyPair, pq: &PqPreKeyPair) -> (DhKeyPair, qt_core::identity::BundleV4) {
    let identity = DhKeyPair::generate();
    let signing = SigningKeyPair::generate();
    let bytes = qt_core::identity::encode_bundle_v4(
        &identity.public,
        &signing,
        5,
        &signed_pre_key.public,
        1_790_000_000,
        2,
        &pq.public_key_bytes(),
    )
    .unwrap();
    (identity, qt_core::identity::parse_bundle_v4(&bytes).unwrap())
}

#[test]
fn v2_os_dois_lados_chegam_ao_mesmo_segredo_com_e_sem_opk() {
    use qt_core::pqxdh::{pqxdh_initiate_v2, pqxdh_respond_v2};
    let alice = DhKeyPair::generate();
    let spk = DhKeyPair::generate();
    let pq = PqPreKeyPair::generate();
    let (bob, bundle) = bob_v4(&spk, &pq);
    for opk in [None, Some(DhKeyPair::generate())] {
        let init = pqxdh_initiate_v2(&alice, &bundle, opk.as_ref().map(|k| &k.public)).unwrap();
        assert_eq!((init.signed_pre_key_id, init.pq_pre_key_id), (5, 2));
        let bob_secret = pqxdh_respond_v2(&bob, &spk, &pq, opk.as_ref(), &alice.public, &init.ephemeral_public, &init.pq_ciphertext).unwrap();
        assert_eq!(init.shared_secret, bob_secret);
    }
}

#[test]
fn v2_difere_do_v1_e_depende_de_cada_chave_da_transcricao() {
    use qt_core::pqxdh::{pqxdh_initiate_v2, pqxdh_respond_v2};
    let alice = DhKeyPair::generate();
    let spk = DhKeyPair::generate();
    let pq = PqPreKeyPair::generate();
    let (bob, bundle) = bob_v4(&spk, &pq);
    let opk = DhKeyPair::generate();
    let init = pqxdh_initiate_v2(&alice, &bundle, Some(&opk.public)).unwrap();
    // A mesma troca com a receita antiga (v1) da outro segredo.
    let v1 = pqxdh_respond(&bob, &spk, &pq, Some(&opk), &alice.public, &init.ephemeral_public, &init.pq_ciphertext).unwrap();
    assert_ne!(v1, init.shared_secret);
    // Sem a OPK que o iniciador usou: segredo diferente.
    let sem_opk = pqxdh_respond_v2(&bob, &spk, &pq, None, &alice.public, &init.ephemeral_public, &init.pq_ciphertext).unwrap();
    assert_ne!(sem_opk, init.shared_secret);
    // Outra prekey ML-KEM do lado de quem responde: nao decifra o mesmo segredo.
    let outra_pq = PqPreKeyPair::generate();
    let r = pqxdh_respond_v2(&bob, &spk, &outra_pq, Some(&opk), &alice.public, &init.ephemeral_public, &init.pq_ciphertext);
    assert!(r.map(|s| s != init.shared_secret).unwrap_or(true));
}

#[test]
fn v2_rejeita_bundle_com_assinatura_adulterada() {
    use qt_core::pqxdh::pqxdh_initiate_v2;
    let spk = DhKeyPair::generate();
    let pq = PqPreKeyPair::generate();
    let (_, mut bundle) = bob_v4(&spk, &pq);
    bundle.created_at += 1;
    assert!(pqxdh_initiate_v2(&DhKeyPair::generate(), &bundle, None).is_err());
}
