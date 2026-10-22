//! Apagar a conta (`DeleteAccount`): o relay esquece o bundle e a fila, e recusa
//! mensagens para quem apagou a conta. Precisa de um Redis em 127.0.0.1:6379.

mod common;
use common::*;

use qt_core::primitives::SigningKeyPair;
use qt_server::client::{RelayClient, RelayConfig, RelayError};
use qt_server::protocol::*;
use qt_server::ServerConfig;
use qt_transport::generate_static_keypair;

fn uid(s: &SigningKeyPair) -> String {
    user_id_from_signing_public(&s.verifying_key.to_bytes())
}

async fn client(relay: &TestRelay, s: &SigningKeyPair) -> RelayClient {
    let n = generate_static_keypair().unwrap();
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    RelayClient::connect(&relay.url, n.private, s.to_bytes(), cfg).await.unwrap()
}

#[tokio::test]
async fn apagar_a_conta_apaga_bundle_e_fila_e_recusa_mensagens_novas() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let (alice, bob) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut a = client(&relay, &alice).await;
    let mut b = client(&relay, &bob).await;
    let bob_id = uid(&bob);

    b.register_bundle(valid_bundle(&bob, 1)).await.unwrap();
    a.register_bundle(valid_bundle(&alice, 2)).await.unwrap();
    a.send_message(&bob_id, vec![], b"antes".to_vec()).await.unwrap();
    assert_eq!(b.fetch_messages().await.unwrap().len(), 1);

    b.delete_account().await.unwrap();

    assert_eq!(a.fetch_bundle(&bob_id).await.unwrap(), None, "o bundle foi apagado");
    assert!(b.fetch_messages().await.unwrap().is_empty(), "a fila foi apagada");
    match a.send_message(&bob_id, vec![], b"depois".to_vec()).await {
        Err(RelayError::Rejected(r)) => assert_eq!(r, RECIPIENT_DELETED),
        other => panic!("devia recusar: {other:?}"),
    }
    // Mensagens para outras pessoas continuam a funcionar.
    let carol = SigningKeyPair::generate();
    registar(&relay, &uid(&carol)).await;
    a.send_message(&uid(&carol), vec![], b"ola".to_vec()).await.unwrap();
}

#[tokio::test]
async fn so_se_apaga_a_propria_conta() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let (bob, eve) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut b = client(&relay, &bob).await;
    let bundle = valid_bundle(&bob, 7);
    b.register_bundle(bundle.clone()).await.unwrap();

    let mut e = client(&relay, &eve).await;
    let r = e.request(ClientMessage::DeleteAccount { user_id: uid(&bob) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
    assert_eq!(e.fetch_bundle(&uid(&bob)).await.unwrap(), Some(bundle), "o bundle do Bob ficou");
}
