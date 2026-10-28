//! `AdminOnline`: so o operador pergunta; conta pessoas (contas diferentes), nao ligacoes,
//! e nao conta o operador. Ficheiro a parte: o registo de ligacoes e do processo inteiro.
//! Precisa de um Redis em 127.0.0.1:6379.

mod common;
use common::*;

use qt_core::primitives::SigningKeyPair;
use qt_core::report;
use qt_server::client::{RelayClient, RelayConfig};
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

async fn online(op: &mut RelayClient) -> u64 {
    match op.request(ClientMessage::AdminOnline).await.unwrap() {
        ServerMessage::Online { users } => users,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn online_conta_pessoas_sem_o_operador() {
    let operator = SigningKeyPair::from_bytes(&report::operator_signing_private(&[7u8; 32]));
    let relay = TestRelay::start(ServerConfig { operator: Some(uid(&operator)), ..ServerConfig::default() }).await;
    let (alice, bob) = (SigningKeyPair::generate(), SigningKeyPair::generate());

    // A Alice com tres ligacoes (como a app) e o Bob com uma: duas pessoas.
    let mut a1 = client(&relay, &alice).await;
    let a2 = client(&relay, &alice).await;
    let a3 = client(&relay, &alice).await;
    let b = client(&relay, &bob).await;

    // So o operador pergunta.
    let r = a1.request(ClientMessage::AdminOnline).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == NOT_OPERATOR), "{r:?}");

    let mut op = client(&relay, &operator).await;
    assert_eq!(online(&mut op).await, 2);

    // O Bob sai: fica a Alice.
    drop(b);
    let mut n = 0;
    for _ in 0..50 {
        n = online(&mut op).await;
        if n == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(n, 1);
    drop((a1, a2, a3));
}
