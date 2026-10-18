//! Limites contra abusos (`limits.rs`): ligacoes por autenticar e por conta.
//! Precisa de um Redis em 127.0.0.1:6379.
//!
//! Um so teste, com os dois casos seguidos: o contador de ligacoes por autenticar
//! e global ao processo, e dois testes em paralelo interfeririam um com o outro.

mod common;
use common::*;

use qt_core::primitives::SigningKeyPair;
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::ServerConfig;
use qt_transport::generate_static_keypair;
use std::time::Duration;

async fn client(relay: &TestRelay, s: &SigningKeyPair) -> Result<RelayClient, qt_server::client::RelayError> {
    let n = generate_static_keypair().unwrap();
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    RelayClient::connect(&relay.url, n.private, s.to_bytes(), cfg).await
}

#[tokio::test]
async fn limites_por_conta_e_por_autenticar() {
    // ---- 1) Por conta: acima do maximo fecha a ligacao mais antiga, a nova fica ----
    let relay = TestRelay::start(ServerConfig { max_connections_per_user: 2, ..ServerConfig::default() }).await;
    let alice = SigningKeyPair::generate();
    let mut c1 = client(&relay, &alice).await.unwrap();
    let mut c2 = client(&relay, &alice).await.unwrap();
    let mut c3 = client(&relay, &alice).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(c1.fetch_messages().await.is_err(), "a mais antiga foi fechada");
    assert!(c2.fetch_messages().await.is_ok());
    assert!(c3.fetch_messages().await.is_ok());
    // Outra conta nao e afetada pelas ligacoes da Alice.
    let mut b = client(&relay, &SigningKeyPair::generate()).await.unwrap();
    assert!(b.fetch_messages().await.is_ok());
    drop((c1, c2, c3, b));
    drop(relay);
    tokio::time::sleep(Duration::from_millis(200)).await;

    // ---- 2) Por autenticar: ligacoes caladas nao tiram a vaga a quem ja esta autenticado ----
    let cfg = ServerConfig { max_unauthenticated: 2, handshake_timeout: Duration::from_secs(3), ..ServerConfig::default() };
    let relay = TestRelay::start(cfg).await;
    let mut ja_autenticado = client(&relay, &SigningKeyPair::generate()).await.unwrap();

    let caladas: Vec<_> = futures_util::future::join_all(
        (0..2).map(|_| tokio::net::TcpStream::connect(("127.0.0.1", relay.port))),
    )
    .await
    .into_iter()
    .map(|r| r.unwrap())
    .collect();
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(client(&relay, &SigningKeyPair::generate()).await.is_err(), "sem vagas por autenticar: recusada");
    assert!(ja_autenticado.fetch_messages().await.is_ok(), "quem ja estava autenticado continua a funcionar");

    drop(caladas);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut nova = client(&relay, &SigningKeyPair::generate()).await.expect("com vagas livres volta a aceitar");
    assert!(nova.fetch_messages().await.is_ok());
}
