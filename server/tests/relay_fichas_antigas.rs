//! Apps 0.9.37 a 0.9.52 com o relay 0.9.53: o pedido das fichas do video (`IssueFichas`, que
//! saiu) recebe sempre o erro que essas apps ja tratam (`FICHAS_OFF`: voltam a perguntar so
//! horas depois) e a ligacao continua a servir os outros pedidos.
//! Precisa de um Redis em 127.0.0.1:6379.

mod common;
use common::*;

use qt_core::primitives::SigningKeyPair;
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::protocol::*;
use qt_server::ServerConfig;
use qt_transport::generate_static_keypair;

#[tokio::test]
async fn pedido_de_fichas_de_uma_app_antiga_recebe_fichas_off() {
    // O JSON e o mesmo que as apps antigas enviam.
    let msg = ClientMessage::IssueFichas { epoch: 202610, blinded: "AAAA".into() };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.starts_with(r#"{"IssueFichas":{"epoch":202610,"blinded":"#), "{json}");
    // E um pedido sem os campos (ou com outros) tambem e lido.
    assert!(matches!(deserialize_client_message(br#"{"IssueFichas":{}}"#), Ok(ClientMessage::IssueFichas { .. })));

    let relay = TestRelay::start(ServerConfig::default()).await;
    let alice = SigningKeyPair::generate();
    let n = generate_static_keypair().unwrap();
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    let mut c = RelayClient::connect(&relay.url, n.private, alice.to_bytes(), cfg).await.unwrap();
    for _ in 0..3 {
        let r = c.request(msg.clone()).await.unwrap();
        assert!(matches!(r, ServerMessage::Error { ref reason } if reason == FICHAS_OFF), "{r:?}");
    }
    // A mesma ligacao continua a responder.
    let r = c.request(ClientMessage::AdminOnline).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == NOT_OPERATOR), "{r:?}");
}
