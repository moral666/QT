//! Teste de ponta a ponta: Alice e Bob estabelecem sessao E2EE (X3DH +
//! Double Ratchet, de `secure_messenger_core`) e depois trocam uma mensagem
//! cifrada atraves de uma ligacao WebSocket REAL em localhost, protegida por
//! um canal Noise (`secure_messenger_transport`).
//!
//! Isto demonstra as DUAS camadas de seguranca definidas na arquitetura:
//!   1. E2EE de aplicacao (Double Ratchet) - o servidor/relay nunca veria o
//!      conteudo em texto plano, mesmo que estivesse no meio.
//!   2. Seguranca de transporte (Noise) - autentica a ligacao e acrescenta
//!      uma segunda camada de cifra independente da primeira.
//!
//! O bytes que viajam pela rede sao: Double-Ratchet-ciphertext, depois
//! embrulhados outra vez pelo Noise. Um observador de rede real (sem as
//! chaves de nenhuma das camadas) nao veria absolutamente nada legivel.

use secure_messenger_core::primitives::{DhKeyPair, SigningKeyPair};
use secure_messenger_core::ratchet::RatchetState;
use secure_messenger_core::x3dh::{sign_pre_key, x3dh_initiate, x3dh_respond, PreKeyBundle};
use secure_messenger_transport::{generate_static_keypair, ws_transport};
use tokio::net::TcpListener;

#[tokio::test]
async fn mensagem_e2ee_atraves_de_websocket_real() {
    // ---------- 1. Estabelecer sessao E2EE (identico ao teste de core/) ----------
    let alice_identity = DhKeyPair::generate();
    let bob_identity = DhKeyPair::generate();
    let bob_identity_signing = SigningKeyPair::generate();
    let bob_signed_pre_key = DhKeyPair::generate();
    let bob_one_time_pre_key = DhKeyPair::generate();

    let signature = sign_pre_key(&bob_identity_signing, &bob_signed_pre_key.public);
    let bob_bundle = PreKeyBundle {
        identity_key: bob_identity.public,
        identity_signing_key: bob_identity_signing.verifying_key,
        signed_pre_key: bob_signed_pre_key.public,
        signed_pre_key_signature: signature,
        one_time_pre_key: Some(bob_one_time_pre_key.public),
    };

    let init_result = x3dh_initiate(&alice_identity, &bob_bundle).unwrap();
    let bob_shared_secret = x3dh_respond(
        &bob_identity,
        &bob_signed_pre_key,
        Some(&bob_one_time_pre_key),
        &alice_identity.public,
        &init_result.ephemeral_public,
    );
    assert_eq!(init_result.shared_secret, bob_shared_secret);

    let mut alice_ratchet =
        RatchetState::init_as_initiator(init_result.shared_secret, bob_signed_pre_key.public);
    let mut bob_ratchet = RatchetState::init_as_responder(bob_shared_secret, bob_signed_pre_key);

    // A mensagem original, em texto simples, que queremos entregar a Bob.
    let mensagem_original = b"Ola Bob! Isto viajou por um WebSocket real, com duas camadas de cifra.";

    // Double Ratchet cifra a mensagem - ISTO e o que um servidor curioso
    // veria (ainda sem o Noise por cima).
    let double_ratchet_ciphertext = alice_ratchet.encrypt(mensagem_original).unwrap();

    // Serializar o EncryptedMessage do ratchet para bytes crus, para poder
    // viajar pela rede. Formato simples: [32 bytes chave DH][4 bytes n][resto = ciphertext].
    let mut wire_bytes = Vec::new();
    wire_bytes.extend_from_slice(double_ratchet_ciphertext.dh_public.as_bytes());
    wire_bytes.extend_from_slice(&double_ratchet_ciphertext.n.to_be_bytes());
    wire_bytes.extend_from_slice(&double_ratchet_ciphertext.ciphertext);

    // ---------- 2. Subir um servidor WebSocket real em localhost ----------
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_static_keys = generate_static_keypair().unwrap();
    let server_static_private = server_static_keys.private.clone();

    // Guarda o que o "servidor" (representando o lado de Bob) efetivamente
    // recebeu, para o teste poder verificar depois.
    let received = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));
    let received_clone = received.clone();

    let server_task = tokio::spawn(async move {
        let (tcp_stream, _peer_addr) = listener.accept().await.unwrap();
        let (mut ws_stream, mut noise_transport) =
            ws_transport::server_accept(tcp_stream, &server_static_private)
                .await
                .expect("handshake Noise (servidor) falhou");

        let payload = ws_transport::receive_encrypted(&mut ws_stream, &mut noise_transport)
            .await
            .expect("falha ao receber/decifrar via Noise");

        *received_clone.lock().await = payload;
    });

    // ---------- 3. Cliente liga-se e envia os bytes ja cifrados pelo Double Ratchet ----------
    let client_static_keys = generate_static_keypair().unwrap();
    let url = format!("ws://{}", addr);

    let (mut ws_stream, mut noise_transport) =
        ws_transport::client_connect(&url, &client_static_keys.private)
            .await
            .expect("handshake Noise (cliente) falhou");

    ws_transport::send_encrypted(&mut ws_stream, &mut noise_transport, &wire_bytes)
        .await
        .expect("falha ao enviar/cifrar via Noise");

    server_task.await.unwrap();

    // ---------- 4. Verificar do lado de "Bob": desfazer as duas camadas ----------
    let bytes_recebidos = received.lock().await.clone();
    assert_eq!(
        bytes_recebidos, wire_bytes,
        "os bytes recebidos pelo servidor devem ser identicos aos enviados, apos o Noise decifrar"
    );

    // Desserializar de volta para um EncryptedMessage e aplicar o Double Ratchet de Bob.
    let dh_public_bytes: [u8; 32] = bytes_recebidos[0..32].try_into().unwrap();
    let n = u32::from_be_bytes(bytes_recebidos[32..36].try_into().unwrap());
    let ciphertext = bytes_recebidos[36..].to_vec();

    let msg_reconstruida = secure_messenger_core::ratchet::EncryptedMessage {
        dh_public: x25519_dalek::PublicKey::from(dh_public_bytes),
        n,
        ciphertext,
    };

    let mensagem_final = bob_ratchet.decrypt(&msg_reconstruida).unwrap();
    assert_eq!(
        mensagem_final, mensagem_original,
        "a mensagem final, depois de atravessar Noise + WebSocket real + Double Ratchet, deve bater certo com a original"
    );
}
