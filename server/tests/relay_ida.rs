//! Tempo de ida (telemovel -> relay): depois de aceitar uma mensagem, o relay manda
//! um ping na mesma ligacao e conta metade da ida e volta. Precisa de um Redis em
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
async fn mensagem_enviada_mede_a_ida_ate_ao_relay() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let (alice, bob) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let n = generate_static_keypair().unwrap();
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    let mut a = RelayClient::connect(&relay.url, n.private, alice.to_bytes(), cfg).await.unwrap();
    let bob_id = user_id_from_signing_public(&bob.verifying_key.to_bytes());

    // Pedidos que nao sao mensagens nao medem nada.
    a.fetch_messages().await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (_, ida) = qt_server::latency::roll_minute();
    assert_eq!(ida.messages, 0);

    for i in 0..3u8 {
        a.send_message(&bob_id, vec![], vec![i]).await.unwrap();
        // O cliente responde ao ping quando le a ligacao (aqui, no pedido seguinte).
        a.fetch_messages().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (entrega, ida) = qt_server::latency::roll_minute();
    assert_eq!(ida.messages, 3, "uma amostra por mensagem enviada");
    assert!(ida.avg_ms.unwrap() < 1000, "{ida:?}");
    assert_eq!(entrega.messages, 0, "ninguem confirmou: a entrega nao conta");
}
