use secure_messenger_core::primitives::{DhKeyPair, SigningKeyPair};
use secure_messenger_core::ratchet::RatchetState;
use secure_messenger_core::x3dh::{sign_pre_key, x3dh_initiate, x3dh_respond, PreKeyBundle};

struct BobKeys {
    identity: DhKeyPair,
    identity_signing: SigningKeyPair,
    signed_pre_key: DhKeyPair,
    one_time_pre_key: DhKeyPair,
}

fn setup_bob() -> BobKeys {
    BobKeys {
        identity: DhKeyPair::generate(),
        identity_signing: SigningKeyPair::generate(),
        signed_pre_key: DhKeyPair::generate(),
        one_time_pre_key: DhKeyPair::generate(),
    }
}

fn bundle_for(bob: &BobKeys) -> PreKeyBundle {
    let signature = sign_pre_key(&bob.identity_signing, &bob.signed_pre_key.public);
    PreKeyBundle {
        identity_key: bob.identity.public,
        identity_signing_key: bob.identity_signing.verifying_key,
        signed_pre_key: bob.signed_pre_key.public,
        signed_pre_key_signature: signature,
        one_time_pre_key: Some(bob.one_time_pre_key.public),
    }
}

#[test]
fn fluxo_completo_alice_bob() {
    let alice_identity = DhKeyPair::generate();
    let bob = setup_bob();
    let bob_bundle = bundle_for(&bob);

    let init_result = x3dh_initiate(&alice_identity, &bob_bundle)
        .expect("assinatura da signed pre-key deveria ser valida");

    let bob_shared_secret = x3dh_respond(
        &bob.identity,
        &bob.signed_pre_key,
        Some(&bob.one_time_pre_key),
        &alice_identity.public,
        &init_result.ephemeral_public,
    );

    assert_eq!(
        init_result.shared_secret, bob_shared_secret,
        "X3DH: segredos compartilhados devem coincidir em ambos os lados"
    );

    let mut alice_state =
        RatchetState::init_as_initiator(init_result.shared_secret, bob.signed_pre_key.public);
    let mut bob_state = RatchetState::init_as_responder(bob_shared_secret, bob.signed_pre_key);

    let mensagens = ["Ola Bob, tudo bem?", "Assinatura + one-time key ja funcionam.", "Terceira mensagem."];
    let mut enviados = Vec::new();
    for m in &mensagens {
        enviados.push(alice_state.encrypt(m.as_bytes()).expect("falha ao cifrar"));
    }
    for (i, msg) in enviados.iter().enumerate() {
        let plaintext = bob_state.decrypt(msg).expect("Bob falhou ao decifrar");
        assert_eq!(plaintext, mensagens[i].as_bytes());
    }

    let resposta = bob_state
        .encrypt(b"Recebi tudo, forward secrecy ok!")
        .expect("falha ao cifrar resposta");
    let resposta_decifrada = alice_state.decrypt(&resposta).expect("Alice falhou ao decifrar");
    assert_eq!(resposta_decifrada, b"Recebi tudo, forward secrecy ok!");

    let m1 = alice_state.encrypt(b"mensagem A").unwrap();
    let m2 = alice_state.encrypt(b"mensagem B").unwrap();
    let dec_m2 = bob_state.decrypt(&m2).expect("falha ao decifrar m2 (fora de ordem)");
    let dec_m1 = bob_state.decrypt(&m1).expect("falha ao decifrar m1 (atrasada)");
    assert_eq!(dec_m2, b"mensagem B");
    assert_eq!(dec_m1, b"mensagem A");
}

#[test]
fn assinatura_adulterada_deve_falhar() {
    let alice_identity = DhKeyPair::generate();
    let bob = setup_bob();
    let mut bundle = bundle_for(&bob);

    // Simula um servidor malicioso substituindo a signed pre-key depois de
    // assinada - a assinatura original deixa de bater com a nova chave.
    bundle.signed_pre_key = DhKeyPair::generate().public;

    let result = x3dh_initiate(&alice_identity, &bundle);
    assert!(
        result.is_err(),
        "x3dh_initiate deveria rejeitar uma signed pre-key com assinatura invalida"
    );
}

#[test]
fn mensagem_adulterada_deve_falhar() {
    let alice_identity = DhKeyPair::generate();
    let bob = setup_bob();
    let bob_bundle = bundle_for(&bob);

    let init_result = x3dh_initiate(&alice_identity, &bob_bundle).unwrap();
    let bob_shared_secret = x3dh_respond(
        &bob.identity,
        &bob.signed_pre_key,
        Some(&bob.one_time_pre_key),
        &alice_identity.public,
        &init_result.ephemeral_public,
    );

    let mut alice_state =
        RatchetState::init_as_initiator(init_result.shared_secret, bob.signed_pre_key.public);
    let mut bob_state = RatchetState::init_as_responder(bob_shared_secret, bob.signed_pre_key);

    let mut msg = alice_state.encrypt(b"mensagem original").unwrap();
    let last = msg.ciphertext.len() - 1;
    msg.ciphertext[last] ^= 0xFF;

    let result = bob_state.decrypt(&msg);
    assert!(result.is_err(), "decifragem de mensagem adulterada deveria falhar");
}

#[test]
fn ratchet_sobrevive_export_e_import() {
    // Prova que uma sessao pode ser guardada (ex.: em SQLCipher, ver
    // storage/) e recarregada, continuando a conversa exatamente do ponto
    // onde ficou - essencial para persistencia entre execucoes do cliente.
    let alice_identity = DhKeyPair::generate();
    let bob = setup_bob();
    let bob_bundle = bundle_for(&bob);

    let init_result = x3dh_initiate(&alice_identity, &bob_bundle).unwrap();
    let bob_shared_secret = x3dh_respond(
        &bob.identity,
        &bob.signed_pre_key,
        Some(&bob.one_time_pre_key),
        &alice_identity.public,
        &init_result.ephemeral_public,
    );

    let mut alice_state =
        RatchetState::init_as_initiator(init_result.shared_secret, bob.signed_pre_key.public);
    let mut bob_state = RatchetState::init_as_responder(bob_shared_secret, bob.signed_pre_key);

    // Troca inicial, antes de "desligar" o processo.
    let msg1 = alice_state.encrypt(b"mensagem antes de guardar").unwrap();
    assert_eq!(bob_state.decrypt(&msg1).unwrap(), b"mensagem antes de guardar");

    // "Desligar": serializar ambos os lados, largar os originais.
    let alice_bytes = alice_state.to_bytes();
    let bob_bytes = bob_state.to_bytes();
    drop(alice_state);
    drop(bob_state);

    // "Religar": reconstruir a partir dos bytes guardados.
    let mut alice_recarregada = RatchetState::from_bytes(&alice_bytes).expect("deveria desserializar");
    let mut bob_recarregado = RatchetState::from_bytes(&bob_bytes).expect("deveria desserializar");

    // A conversa continua exatamente de onde ficou.
    let msg2 = alice_recarregada.encrypt(b"mensagem depois de recarregar").unwrap();
    assert_eq!(
        bob_recarregado.decrypt(&msg2).unwrap(),
        b"mensagem depois de recarregar"
    );

    let resposta = bob_recarregado.encrypt(b"e a resposta tambem funciona").unwrap();
    assert_eq!(
        alice_recarregada.decrypt(&resposta).unwrap(),
        b"e a resposta tambem funciona"
    );
}

#[test]
fn sem_one_time_pre_key_ainda_funciona() {
    // Garante que o protocolo degrada de forma graciosa quando o servidor
    // ficou sem one-time pre-keys disponiveis (cenario realista em producao).
    let alice_identity = DhKeyPair::generate();
    let bob = setup_bob();
    let mut bundle = bundle_for(&bob);
    bundle.one_time_pre_key = None;

    let init_result = x3dh_initiate(&alice_identity, &bundle).unwrap();
    let bob_shared_secret = x3dh_respond(
        &bob.identity,
        &bob.signed_pre_key,
        None,
        &alice_identity.public,
        &init_result.ephemeral_public,
    );

    assert_eq!(init_result.shared_secret, bob_shared_secret);
}
