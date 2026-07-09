//! Prova a capacidade especifica do servidor/relay: ENTREGA ASSINCRONA.
//! Alice liga-se, envia uma mensagem para Bob, e desliga-se. So DEPOIS
//! disso e que Bob se liga (numa ligacao TCP/WebSocket completamente
//! separada) e recebe a mensagem que estava a espera dele na fila.
//!
//! O servidor nunca ve o conteudo: o `ciphertext` que ele guarda e
//! exatamente o que sai do Double Ratchet em `core/` - bytes opacos.

use secure_messenger_core::primitives::{DhKeyPair, SigningKeyPair};
use secure_messenger_core::ratchet::{EncryptedMessage, RatchetState};
use secure_messenger_core::x3dh::{sign_pre_key, x3dh_initiate, x3dh_respond, PreKeyBundle};
use secure_messenger_server::protocol::{
    deserialize_server_message, serialize_client_message, ClientMessage, ServerMessage,
};
use secure_messenger_server::Store;
use secure_messenger_transport::{generate_static_keypair, ws_transport};
use std::sync::Arc;
use tokio::net::TcpListener;

/// Sobe uma instancia do servidor/relay real em localhost, numa porta
/// aleatoria livre, e devolve o endereco a que os clientes se devem ligar.
async fn spawn_test_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = Arc::new(Store::new());
    let static_keys = generate_static_keypair().unwrap();

    tokio::spawn(async move {
        loop {
            let (tcp_stream, _peer) = match listener.accept().await {
                Ok(x) => x,
                Err(_) => break,
            };
            let store = store.clone();
            let static_private = static_keys.private.clone();
            tokio::spawn(async move {
                let _ = secure_messenger_server::handle_connection(tcp_stream, store, static_private)
                    .await;
            });
        }
    });

    format!("ws://{}", addr)
}

/// Helper: liga-se ao servidor, envia UMA ClientMessage, recebe UMA
/// ServerMessage de resposta, e fecha a ligacao. Suficiente para este teste,
/// onde cada "sessao" de cliente faz so uma operacao antes de desligar.
async fn send_one_message(url: &str, msg: ClientMessage) -> ServerMessage {
    let client_keys = generate_static_keypair().unwrap();
    let (mut ws_stream, mut noise_transport) =
        ws_transport::client_connect(url, &client_keys.private).await.unwrap();

    let bytes = serialize_client_message(&msg);
    ws_transport::send_encrypted(&mut ws_stream, &mut noise_transport, &bytes).await.unwrap();

    let response_bytes =
        ws_transport::receive_encrypted(&mut ws_stream, &mut noise_transport).await.unwrap();
    deserialize_server_message(&response_bytes).unwrap()
}

#[tokio::test]
async fn entrega_assincrona_atraves_do_relay() {
    // ---------- Sessao E2EE entre Alice e Bob (igual aos testes anteriores) ----------
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

    let mut alice_ratchet =
        RatchetState::init_as_initiator(init_result.shared_secret, bob_signed_pre_key.public);
    let mut bob_ratchet = RatchetState::init_as_responder(bob_shared_secret, bob_signed_pre_key);

    let mensagem_original = b"Bob, esta mensagem esperou por ti na fila do servidor.";
    let encrypted = alice_ratchet.encrypt(mensagem_original).unwrap();

    let mut wire_bytes = Vec::new();
    wire_bytes.extend_from_slice(encrypted.dh_public.as_bytes());
    wire_bytes.extend_from_slice(&encrypted.n.to_be_bytes());
    wire_bytes.extend_from_slice(&encrypted.ciphertext);

    // ---------- Servidor real em localhost ----------
    let server_url = spawn_test_server().await;

    // ---------- Alice liga-se, envia, e DESLIGA-SE (Bob nao esta online) ----------
    let response = send_one_message(
        &server_url,
        ClientMessage::SendMessage { from: "alice".to_string(), to: "bob".to_string(), ciphertext: wire_bytes.clone() },
    )
    .await;
    assert!(matches!(response, ServerMessage::Ack), "servidor deveria confirmar o envio");

    // ---------- So agora Bob se liga (ligacao TCP completamente separada) ----------
    let response = send_one_message(
        &server_url,
        ClientMessage::FetchMessages { user_id: "bob".to_string() },
    )
    .await;

    let messages = match response {
        ServerMessage::Messages { messages } => messages,
        other => panic!("esperava ServerMessage::Messages, recebi {other:?}"),
    };
    assert_eq!(messages.len(), 1, "Bob deveria ter exatamente 1 mensagem em fila");
    assert_eq!(messages[0].from, "alice");
    assert_eq!(messages[0].ciphertext, wire_bytes);

    // Segunda tentativa de fetch deve vir vazia - a fila foi drenada.
    let response = send_one_message(
        &server_url,
        ClientMessage::FetchMessages { user_id: "bob".to_string() },
    )
    .await;
    match response {
        ServerMessage::Messages { messages } => {
            assert!(messages.is_empty(), "a fila ja deveria estar vazia apos o primeiro fetch")
        }
        other => panic!("esperava ServerMessage::Messages, recebi {other:?}"),
    }

    // ---------- Bob desfaz as camadas: Noise/servidor ja tratado, falta Double Ratchet ----------
    let dh_public_bytes: [u8; 32] = messages[0].ciphertext[0..32].try_into().unwrap();
    let n = u32::from_be_bytes(messages[0].ciphertext[32..36].try_into().unwrap());
    let ratchet_ciphertext = messages[0].ciphertext[36..].to_vec();

    let msg_reconstruida = EncryptedMessage {
        dh_public: x25519_dalek::PublicKey::from(dh_public_bytes),
        n,
        ciphertext: ratchet_ciphertext,
    };
    let mensagem_final = bob_ratchet.decrypt(&msg_reconstruida).unwrap();
    assert_eq!(mensagem_final, mensagem_original);
}

#[tokio::test]
async fn registo_e_consulta_de_prekey_bundle() {
    let server_url = spawn_test_server().await;

    let bundle_falso = b"bytes-opacos-representando-um-PreKeyBundle-serializado".to_vec();

    let response = send_one_message(
        &server_url,
        ClientMessage::RegisterPreKeyBundle {
            user_id: "alice".to_string(),
            bundle_bytes: bundle_falso.clone(),
        },
    )
    .await;
    assert!(matches!(response, ServerMessage::Ack));

    let response = send_one_message(
        &server_url,
        ClientMessage::FetchPreKeyBundle { user_id: "alice".to_string() },
    )
    .await;
    match response {
        ServerMessage::PreKeyBundle { bundle_bytes } => assert_eq!(bundle_bytes, bundle_falso),
        other => panic!("esperava PreKeyBundle, recebi {other:?}"),
    }

    let response = send_one_message(
        &server_url,
        ClientMessage::FetchPreKeyBundle { user_id: "utilizador-que-nao-existe".to_string() },
    )
    .await;
    assert!(matches!(response, ServerMessage::PreKeyBundleNotFound));
}
