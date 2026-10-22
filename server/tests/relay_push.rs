//! Token de notificacoes (`RegisterPushToken`): so o proprio, validado, apagado com
//! a conta. Precisa de um Redis em 127.0.0.1:6379. (A chamada a Google nao e
//! testada aqui: sem QT_FCM_CREDENTIALS as notificacoes ficam desligadas.)

mod common;
use common::*;

use qt_core::primitives::SigningKeyPair;
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::protocol::*;
use qt_server::{ServerConfig, Store};
use qt_transport::generate_static_keypair;

fn uid(s: &SigningKeyPair) -> String {
    user_id_from_signing_public(&s.verifying_key.to_bytes())
}

/// Conta registada (so contas registadas escrevem no relay).
async fn client(relay: &TestRelay, s: &SigningKeyPair) -> RelayClient {
    registar(relay, &user_id_from_signing_public(&s.verifying_key.to_bytes())).await;
    let n = generate_static_keypair().unwrap();
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    RelayClient::connect(&relay.url, n.private, s.to_bytes(), cfg).await.unwrap()
}

const TOKEN: &str = "fGx1Q2abc:APA91bH-x_y.z";

#[tokio::test]
async fn registar_trocar_apagar_e_apagar_com_a_conta() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let store = Store::with_prefix("redis://127.0.0.1:6379", &relay.prefix).unwrap();
    let bob = SigningKeyPair::generate();
    let mut b = client(&relay, &bob).await;

    let r = b.request(ClientMessage::RegisterPushToken { user_id: uid(&bob), token: TOKEN.into() }).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    assert_eq!(store.get_push_token(&uid(&bob)).await.unwrap().as_deref(), Some(TOKEN));

    // Vazio = deixar de receber.
    let r = b.request(ClientMessage::RegisterPushToken { user_id: uid(&bob), token: String::new() }).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    assert_eq!(store.get_push_token(&uid(&bob)).await.unwrap(), None);

    // Apagar a conta tambem apaga o token.
    b.request(ClientMessage::RegisterPushToken { user_id: uid(&bob), token: TOKEN.into() }).await.unwrap();
    b.delete_account().await.unwrap();
    assert_eq!(store.get_push_token(&uid(&bob)).await.unwrap(), None);
}

#[tokio::test]
async fn so_o_proprio_e_so_tokens_validos() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let store = Store::with_prefix("redis://127.0.0.1:6379", &relay.prefix).unwrap();
    let (bob, eve) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut e = client(&relay, &eve).await;

    // A Eve nao consegue pôr o token dela na conta do Bob (para receber as campainhas dele).
    let r = e.request(ClientMessage::RegisterPushToken { user_id: uid(&bob), token: TOKEN.into() }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
    assert_eq!(store.get_push_token(&uid(&bob)).await.unwrap(), None);

    for bad in ["com espaco", "a/b", "{json}", &"x".repeat(MAX_PUSH_TOKEN + 1)] {
        let r = e.request(ClientMessage::RegisterPushToken { user_id: uid(&eve), token: bad.to_string() }).await.unwrap();
        assert!(matches!(r, ServerMessage::Error { .. }), "{bad}: {r:?}");
    }
    assert_eq!(store.get_push_token(&uid(&eve)).await.unwrap(), None);
}

#[test]
fn o_pedido_da_app_e_o_que_o_relay_espera() {
    let path = format!("/tmp/qt_push_{}.sqlite", std::process::id());
    let _ = std::fs::remove_file(&path);
    let c = qt_ffi::QtClient::open(path.clone(), vec![9; 32]).unwrap();
    c.create_identity().unwrap();
    let req = c.relay_request_push_token(TOKEN.into()).unwrap();
    match deserialize_client_message(&req).unwrap() {
        ClientMessage::RegisterPushToken { token, user_id } => {
            assert_eq!(token, TOKEN);
            assert!(is_valid_user_id(&user_id));
        }
        other => panic!("{other:?}"),
    }
    assert!(c.relay_request_push_token("com espaco".into()).is_err());
    assert!(c.relay_request_push_token(String::new()).is_ok(), "vazio = deixar de receber");
    let _ = std::fs::remove_file(path);
}
