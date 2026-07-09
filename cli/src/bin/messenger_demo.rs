//! Demo de terminal: mostra, passo a passo, uma conversa E2EE completa entre
//! "Alice" e "Bob", usando as pecas reais ja construidas e testadas:
//! core (X3DH + Double Ratchet) + transport (Noise/WebSocket) + server (relay).
//!
//! Uso:
//!   cargo run --bin messenger_demo
//!
//! Isto sobe o seu PROPRIO servidor relay numa porta local aleatoria (para
//! a demo ser auto-contida e correr com um unico comando). Para testar
//! contra um servidor ja a correr separadamente (`cargo run --bin relay_server`),
//! passa o URL:
//!   cargo run --bin messenger_demo -- ws://127.0.0.1:9443
//!
//! LIMITACAO CONHECIDA (documentada, nao escondida): esta demo corre tudo
//! num unico processo e nao persiste nada em disco - cada execucao gera
//! identidades novas. Sessao/identidade persistente entre execucoes
//! separadas do CLI e a proxima peca a construir (precisa de
//! armazenamento local - ver docs/protocol-spec.md secao 5).

use secure_messenger_cli::wire_format;
use secure_messenger_core::primitives::{DhKeyPair, SigningKeyPair};
use secure_messenger_core::ratchet::{EncryptedMessage, RatchetState};
use secure_messenger_core::x3dh::{sign_pre_key, x3dh_initiate, x3dh_respond, PreKeyBundle};
use secure_messenger_server::protocol::{
    deserialize_server_message, serialize_client_message, ClientMessage, ServerMessage,
};
use secure_messenger_server::Store;
use secure_messenger_transport::{generate_static_keypair, ws_transport};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

fn passo(texto: &str) {
    println!("\n>>> {texto}");
}

async fn subir_servidor_local() -> String {
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
                let _ =
                    secure_messenger_server::handle_connection(tcp_stream, store, static_private)
                        .await;
            });
        }
    });

    format!("ws://{addr}")
}

async fn enviar_ao_servidor(url: &str, msg: ClientMessage) -> ServerMessage {
    let client_keys = generate_static_keypair().unwrap();
    let (mut ws_stream, mut noise) =
        ws_transport::client_connect(url, &client_keys.private).await.unwrap();

    let bytes = serialize_client_message(&msg);
    ws_transport::send_encrypted(&mut ws_stream, &mut noise, &bytes).await.unwrap();

    let response_bytes = ws_transport::receive_encrypted(&mut ws_stream, &mut noise).await.unwrap();
    deserialize_server_message(&response_bytes).unwrap()
}

#[tokio::main]
async fn main() {
    println!("=====================================================");
    println!(" Demo: mensagem E2EE real, ponta a ponta");
    println!(" core (X3DH+Ratchet) + transport (Noise/WS) + server (relay)");
    println!("=====================================================");

    let server_url = match std::env::args().nth(1) {
        Some(url) => {
            println!("\nA usar servidor externo: {url}");
            url
        }
        None => {
            passo("A subir um servidor/relay local (porta aleatoria)...");
            let url = subir_servidor_local().await;
            println!("    servidor a correr em: {url}");
            url
        }
    };

    // ---------- Bob gera a sua identidade e publica o bundle de pre-keys ----------
    passo("Bob gera a sua identidade e publica o bundle de pre-keys no servidor...");
    let bob_identity = DhKeyPair::generate();
    let bob_identity_signing = SigningKeyPair::generate();
    let bob_signed_pre_key = DhKeyPair::generate();
    let bob_one_time_pre_key = DhKeyPair::generate();

    let signature = sign_pre_key(&bob_identity_signing, &bob_signed_pre_key.public);
    let bob_bundle_local = PreKeyBundle {
        identity_key: bob_identity.public,
        identity_signing_key: bob_identity_signing.verifying_key,
        signed_pre_key: bob_signed_pre_key.public,
        signed_pre_key_signature: signature,
        one_time_pre_key: Some(bob_one_time_pre_key.public),
    };
    let bundle_bytes = wire_format::serialize_bundle(&bob_bundle_local);

    let resp = enviar_ao_servidor(
        &server_url,
        ClientMessage::RegisterPreKeyBundle { user_id: "bob".into(), bundle_bytes },
    )
    .await;
    println!("    servidor respondeu: {resp:?}");

    // ---------- Alice gera a sua identidade e busca o bundle de Bob ----------
    passo("Alice gera a sua identidade e pede o bundle publico de Bob ao servidor...");
    let alice_identity = DhKeyPair::generate();

    let resp = enviar_ao_servidor(
        &server_url,
        ClientMessage::FetchPreKeyBundle { user_id: "bob".into() },
    )
    .await;
    let bob_bundle = match resp {
        ServerMessage::PreKeyBundle { bundle_bytes } => {
            wire_format::deserialize_bundle(&bundle_bytes).expect("bundle invalido")
        }
        other => panic!("esperava PreKeyBundle, recebi {other:?}"),
    };
    println!("    Alice recebeu o bundle de Bob (e verificou a assinatura da signed pre-key).");

    // ---------- Alice estabelece a sessao E2EE (X3DH) e cifra a mensagem ----------
    passo("Alice faz o handshake X3DH e cifra a mensagem com o Double Ratchet...");
    let init_result = x3dh_initiate(&alice_identity, &bob_bundle)
        .expect("assinatura invalida - handshake abortado");
    let mut alice_ratchet =
        RatchetState::init_as_initiator(init_result.shared_secret, bob_bundle.signed_pre_key);

    let mensagem = "Ola Bob! Isto e uma mensagem real, cifrada ponta-a-ponta.";
    println!("    Alice escreve: \"{mensagem}\"");
    let encrypted = alice_ratchet.encrypt(mensagem.as_bytes()).unwrap();

    let mut wire_bytes = Vec::new();
    wire_bytes.extend_from_slice(encrypted.dh_public.as_bytes());
    wire_bytes.extend_from_slice(&encrypted.n.to_be_bytes());
    wire_bytes.extend_from_slice(&encrypted.ciphertext);
    println!(
        "    (o que viaja pela rede, em hex, primeiros 32 bytes: {})",
        hex_preview(&wire_bytes)
    );

    // ---------- Alice envia ao servidor. O servidor NUNCA ve o texto original. ----------
    passo("Alice envia a mensagem cifrada ao servidor (que so ve bytes opacos)...");
    let resp = enviar_ao_servidor(
        &server_url,
        ClientMessage::SendMessage { from: "alice".into(), to: "bob".into(), ciphertext: wire_bytes },
    )
    .await;
    println!("    servidor respondeu: {resp:?}");

    // ---------- Simula Bob estar offline por um instante ----------
    passo("(Bob estava offline neste momento - a mensagem fica em fila no servidor)");
    tokio::time::sleep(Duration::from_millis(300)).await;

    // ---------- Bob liga-se agora e busca as mensagens em fila ----------
    passo("Bob liga-se agora e pede as mensagens que estavam a espera dele...");
    let resp =
        enviar_ao_servidor(&server_url, ClientMessage::FetchMessages { user_id: "bob".into() })
            .await;
    let messages = match resp {
        ServerMessage::Messages { messages } => messages,
        other => panic!("esperava Messages, recebi {other:?}"),
    };
    println!("    Bob recebeu {} mensagem(ns) da fila, de: {}", messages.len(), messages[0].from);

    // ---------- Bob completa o X3DH do seu lado e decifra ----------
    passo("Bob completa o handshake X3DH e decifra com o Double Ratchet...");
    let bob_shared_secret = x3dh_respond(
        &bob_identity,
        &bob_signed_pre_key,
        Some(&bob_one_time_pre_key),
        &alice_identity.public,
        &init_result.ephemeral_public,
    );
    let mut bob_ratchet = RatchetState::init_as_responder(bob_shared_secret, bob_signed_pre_key);

    let recebido = &messages[0].ciphertext;
    let dh_public_bytes: [u8; 32] = recebido[0..32].try_into().unwrap();
    let n = u32::from_be_bytes(recebido[32..36].try_into().unwrap());
    let ratchet_ciphertext = recebido[36..].to_vec();
    let msg = EncryptedMessage {
        dh_public: x25519_dalek::PublicKey::from(dh_public_bytes),
        n,
        ciphertext: ratchet_ciphertext,
    };
    let mensagem_decifrada = bob_ratchet.decrypt(&msg).unwrap();

    println!(
        "    Bob le: \"{}\"",
        String::from_utf8_lossy(&mensagem_decifrada)
    );

    println!("\n=====================================================");
    if mensagem_decifrada == mensagem.as_bytes() {
        println!(" SUCESSO: a mensagem chegou intacta, com E2EE real,");
        println!(" atraves de um servidor que nunca viu o conteudo.");
    } else {
        println!(" FALHOU: a mensagem nao bateu certo (nao deveria acontecer).");
    }
    println!("=====================================================");
}

fn hex_preview(bytes: &[u8]) -> String {
    bytes.iter().take(32).map(|b| format!("{b:02x}")).collect()
}
