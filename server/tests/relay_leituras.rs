//! Leituras telefone a telefone (`latency::readings`): uma por mensagem normal para
//! quem estava a espera (`WaitMessages`), com a ida de quem enviou e a entrega ate quem
//! recebeu confirmar. Recibos/presenca (`silent`) e mensagens para quem nao estava a
//! espera nao contam. Precisa de um Redis em 127.0.0.1:6379. Ficheiro proprio: o
//! medidor (`latency`) e global ao processo.

mod common;
use common::*;

use qt_core::primitives::SigningKeyPair;
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::protocol::*;
use qt_server::ServerConfig;
use qt_transport::generate_static_keypair;
use std::time::Duration;

async fn ligar(relay: &TestRelay, k: &SigningKeyPair) -> RelayClient {
    let n = generate_static_keypair().unwrap();
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    RelayClient::connect(&relay.url, n.private, k.to_bytes(), cfg).await.unwrap()
}

/// Bob fica a espera numa ligacao propria (como a app aberta); devolve-a com o que chegou.
fn esperar(mut b: RelayClient) -> tokio::task::JoinHandle<(RelayClient, Vec<DeliveredMessage>)> {
    tokio::spawn(async move {
        let msgs = b.wait_messages(20).await.unwrap();
        (b, msgs)
    })
}

async fn ate_estar_a_espera(user_id: &str) {
    for _ in 0..200 {
        if qt_server::waiters::is_waiting(user_id) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("bob nunca ficou a espera");
}

#[tokio::test]
async fn leitura_so_para_mensagens_normais_com_a_app_aberta() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let (alice, bob) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let alice_id = user_id_from_signing_public(&alice.verifying_key.to_bytes());
    let bob_id = user_id_from_signing_public(&bob.verifying_key.to_bytes());
    registar(&relay, &alice_id).await;
    registar(&relay, &bob_id).await;
    let mut a = ligar(&relay, &alice).await;
    let mut b = ligar(&relay, &bob).await;
    let antes = qt_server::latency::readings().len();

    // 1) Bob nao esta a espera (app fechada): entregue, confirmada, mas sem leitura.
    a.send_message(&bob_id, vec![], vec![1]).await.unwrap();
    a.fetch_messages().await.unwrap(); // responde ao ping da ida
    let m = b.fetch_messages().await.unwrap();
    b.ack(m.iter().map(|m| m.id).collect()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(qt_server::latency::readings().len(), antes, "sem ninguem a espera nao ha leitura");

    // 2) Bob a espera, mas e um recibo/presenca (`silent`): sem leitura.
    let w = esperar(b);
    ate_estar_a_espera(&bob_id).await;
    let r = a
        .request(ClientMessage::SendMessage { to: bob_id.clone(), sealed_from: vec![], ciphertext: vec![2], silent: true, wake: None })
        .await
        .unwrap();
    assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    a.fetch_messages().await.unwrap();
    let (mut b, m) = w.await.unwrap();
    assert_eq!(m.len(), 1);
    b.ack(m.iter().map(|m| m.id).collect()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(qt_server::latency::readings().len(), antes, "recibos e presenca nao contam");

    // 3) Bob a espera e uma mensagem normal: uma leitura, com a ida e a entrega.
    let w = esperar(b);
    ate_estar_a_espera(&bob_id).await;
    a.send_message(&bob_id, vec![], vec![3]).await.unwrap();
    let (mut b, m) = w.await.unwrap();
    assert_eq!(m.len(), 1);
    b.ack(m.iter().map(|m| m.id).collect()).await.unwrap();
    // A ida so fica medida quando a Alice responde ao ping (aqui, depois da confirmacao).
    a.fetch_messages().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let rs = qt_server::latency::readings();
    assert_eq!(rs.len(), antes + 1, "{rs:?}");
    let r = rs[0];
    assert!(r.entrega_ms < 5_000 && r.ida_ms < 1_000, "{r:?}");
    assert_eq!(r.total_ms(), r.ida_ms + r.entrega_ms);
    assert_eq!(r.minute_unix % 60, 0, "so o minuto, nunca a hora exata");
}
