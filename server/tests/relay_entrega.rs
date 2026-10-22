//! Tempo de entrega (relay -> destinatario confirmar): so conta quando o
//! destinatario estava ligado com a app aberta. Precisa de um Redis em
//! 127.0.0.1:6379. Ficheiro proprio: o medidor (`latency`) e global ao processo.

mod common;
use common::*;

use qt_core::primitives::SigningKeyPair;
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::protocol::*;
use qt_server::ServerConfig;
use qt_transport::generate_static_keypair;
use std::time::Duration;

#[tokio::test]
async fn entrega_so_conta_com_o_destinatario_ligado() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let (alice, bob) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    let mut a = RelayClient::connect(&relay.url, generate_static_keypair().unwrap().private, alice.to_bytes(), cfg.clone()).await.unwrap();
    let bob_id = user_id_from_signing_public(&bob.verifying_key.to_bytes());
    registar(&relay, &user_id_from_signing_public(&alice.verifying_key.to_bytes())).await;
    registar(&relay, &bob_id).await;
    qt_server::latency::roll_minute();

    // App do Bob fechada: a mensagem espera na fila. Quando ele abre e confirma,
    // o tempo de espera nao conta (mediria a ausencia dele, nao a entrega).
    a.send_message(&bob_id, vec![], vec![1]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut b = RelayClient::connect(&relay.url, generate_static_keypair().unwrap().private, bob.to_bytes(), cfg).await.unwrap();
    let got = b.fetch_messages().await.unwrap();
    assert_eq!(got.len(), 1);
    b.ack(got.iter().map(|m| m.id).collect()).await.unwrap();
    let (entrega, _) = qt_server::latency::roll_minute();
    assert_eq!(entrega.messages, 0, "destinatario desligado: nao conta");

    // App do Bob aberta (a espera de mensagens): conta.
    let espera = tokio::spawn(async move {
        let got = b.wait_messages(10).await.unwrap();
        b.ack(got.iter().map(|m| m.id).collect()).await.unwrap();
        got.len()
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    a.send_message(&bob_id, vec![], vec![2]).await.unwrap();
    assert_eq!(espera.await.unwrap(), 1);
    let (entrega, _) = qt_server::latency::roll_minute();
    assert_eq!(entrega.messages, 1, "destinatario ligado: conta");
    assert!(entrega.median_ms.unwrap() < 2000, "{entrega:?}");
}
