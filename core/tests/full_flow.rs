use secure_messenger_core::primitives::DhKeyPair;
use secure_messenger_core::ratchet::RatchetState;
use secure_messenger_core::x3dh::{x3dh_initiate, x3dh_respond, PreKeyBundle};

#[test]
fn fluxo_completo_alice_bob() {
    // --- Setup de identidades ---
    let alice_identity = DhKeyPair::generate();
    let bob_identity = DhKeyPair::generate();
    let bob_signed_pre_key = DhKeyPair::generate();

    let bob_bundle = PreKeyBundle {
        identity_key: bob_identity.public,
        signed_pre_key: bob_signed_pre_key.public,
    };

    // --- X3DH ---
    let init_result = x3dh_initiate(&alice_identity, &bob_bundle);
    let bob_shared_secret = x3dh_respond(
        &bob_identity,
        &bob_signed_pre_key,
        &alice_identity.public,
        &init_result.ephemeral_public,
    );
    assert_eq!(
        init_result.shared_secret, bob_shared_secret,
        "X3DH: segredos compartilhados devem coincidir em ambos os lados"
    );

    // --- Inicializacao do Double Ratchet ---
    let mut alice_state =
        RatchetState::init_as_initiator(init_result.shared_secret, bob_signed_pre_key.public);
    // Precisamos reconstruir o par de chaves de Bob para o responder - em
    // producao o "private" nunca seria clonado assim; aqui simulamos
    // reaproveitando o par gerado no teste.
    let mut bob_state = RatchetState::init_as_responder(bob_shared_secret, bob_signed_pre_key);

    // --- Alice envia 3 mensagens ---
    let mensagens = ["Ola Bob, tudo bem?", "Prova de conceito em Rust.", "Terceira mensagem."];
    let mut enviados = Vec::new();
    for m in &mensagens {
        enviados.push(alice_state.encrypt(m.as_bytes()).expect("falha ao cifrar"));
    }
    for (i, msg) in enviados.iter().enumerate() {
        let plaintext = bob_state.decrypt(msg).expect("Bob falhou ao decifrar");
        assert_eq!(plaintext, mensagens[i].as_bytes());
    }

    // --- Bob responde (forca DH ratchet step) ---
    let resposta = bob_state
        .encrypt(b"Recebi tudo, forward secrecy ok!")
        .expect("falha ao cifrar resposta");
    let resposta_decifrada = alice_state.decrypt(&resposta).expect("Alice falhou ao decifrar");
    assert_eq!(resposta_decifrada, b"Recebi tudo, forward secrecy ok!");

    // --- Mensagens fora de ordem (m2 chega antes de m1) ---
    let m1 = alice_state.encrypt(b"mensagem A").unwrap();
    let m2 = alice_state.encrypt(b"mensagem B").unwrap();
    let dec_m2 = bob_state.decrypt(&m2).expect("falha ao decifrar m2 (fora de ordem)");
    let dec_m1 = bob_state.decrypt(&m1).expect("falha ao decifrar m1 (atrasada)");
    assert_eq!(dec_m2, b"mensagem B");
    assert_eq!(dec_m1, b"mensagem A");
}

#[test]
fn mensagem_adulterada_deve_falhar() {
    let alice_identity = DhKeyPair::generate();
    let bob_identity = DhKeyPair::generate();
    let bob_signed_pre_key = DhKeyPair::generate();

    let bob_bundle = PreKeyBundle {
        identity_key: bob_identity.public,
        signed_pre_key: bob_signed_pre_key.public,
    };

    let init_result = x3dh_initiate(&alice_identity, &bob_bundle);
    let bob_shared_secret = x3dh_respond(
        &bob_identity,
        &bob_signed_pre_key,
        &alice_identity.public,
        &init_result.ephemeral_public,
    );

    let mut alice_state =
        RatchetState::init_as_initiator(init_result.shared_secret, bob_signed_pre_key.public);
    let mut bob_state = RatchetState::init_as_responder(bob_shared_secret, bob_signed_pre_key);

    let mut msg = alice_state.encrypt(b"mensagem original").unwrap();
    // Adultera um byte do ciphertext (simula atacante na rede)
    let last = msg.ciphertext.len() - 1;
    msg.ciphertext[last] ^= 0xFF;

    let result = bob_state.decrypt(&msg);
    assert!(result.is_err(), "decifragem de mensagem adulterada deveria falhar");
}
