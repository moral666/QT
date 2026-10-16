//! Entrega imediata (`WaitMessages`): o destinatario fica a espera numa ligacao
//! propria e recebe a mensagem no instante em que ela entra no relay, sem
//! perguntar repetidamente. Precisa de um Redis em 127.0.0.1:6379.

mod common;
use common::*;

use qt_core::primitives::SigningKeyPair;
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::protocol::*;
use qt_server::ServerConfig;
use qt_transport::generate_static_keypair;
use std::time::{Duration, Instant};

fn uid(s: &SigningKeyPair) -> String {
    user_id_from_signing_public(&s.verifying_key.to_bytes())
}

async fn client(relay: &TestRelay, s: &SigningKeyPair) -> RelayClient {
    let n = generate_static_keypair().unwrap();
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    RelayClient::connect(&relay.url, n.private, s.to_bytes(), cfg).await.unwrap()
}

#[tokio::test]
async fn com_mensagens_na_fila_responde_logo() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let (alice, bob) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut a = client(&relay, &alice).await;
    a.send_message(&uid(&bob), vec![], b"ola".to_vec()).await.unwrap();

    let mut b = client(&relay, &bob).await;
    let t = Instant::now();
    let msgs = b.wait_messages(10).await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].ciphertext, b"ola");
    assert!(t.elapsed() < Duration::from_secs(2), "nao devia esperar: {:?}", t.elapsed());
}

#[tokio::test]
async fn fila_vazia_acorda_quando_chega_uma_mensagem() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let (alice, bob) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut b = client(&relay, &bob).await;
    let mut a = client(&relay, &alice).await;
    let to = uid(&bob);

    let espera = tokio::spawn(async move {
        let t = Instant::now();
        let msgs = b.wait_messages(20).await.unwrap();
        (msgs, t.elapsed(), b)
    });
    // Bob ja esta a espera quando a Alice envia.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let enviada = Instant::now();
    a.send_message(&to, vec![], b"agora".to_vec()).await.unwrap();

    let (msgs, esperou, mut b) = espera.await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].ciphertext, b"agora");
    assert!(esperou >= Duration::from_millis(300), "devia ter esperado pela mensagem: {esperou:?}");
    assert!(enviada.elapsed() < Duration::from_secs(2), "entrega devia ser imediata: {:?}", enviada.elapsed());

    // Continua a ser entrega at-least-once: so sai da fila depois do ack.
    assert_eq!(b.fetch_messages().await.unwrap().len(), 1);
    b.ack(vec![msgs[0].id]).await.unwrap();
    assert!(b.fetch_messages().await.unwrap().is_empty());
}

#[tokio::test]
async fn sem_mensagens_devolve_vazio_ao_fim_do_tempo() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let bob = SigningKeyPair::generate();
    let mut b = client(&relay, &bob).await;
    let t = Instant::now();
    assert!(b.wait_messages(1).await.unwrap().is_empty());
    let e = t.elapsed();
    assert!(e >= Duration::from_millis(900) && e < Duration::from_secs(4), "{e:?}");
}

#[tokio::test]
async fn mensagem_para_outra_pessoa_nao_acorda() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let (alice, bob, carol) = (SigningKeyPair::generate(), SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut b = client(&relay, &bob).await;
    let mut a = client(&relay, &alice).await;
    let carol_id = uid(&carol);

    let espera = tokio::spawn(async move {
        let t = Instant::now();
        (b.wait_messages(2).await.unwrap(), t.elapsed())
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    a.send_message(&carol_id, vec![], b"para a carol".to_vec()).await.unwrap();
    let (msgs, esperou) = espera.await.unwrap();
    assert!(msgs.is_empty());
    assert!(esperou >= Duration::from_millis(1900), "acordou sem razao: {esperou:?}");
}

#[tokio::test]
async fn so_espera_pela_propria_fila_e_o_tempo_tem_limite() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let (bob, eve) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut e = client(&relay, &eve).await;
    let r = e
        .request(ClientMessage::WaitMessages { user_id: uid(&bob), timeout_secs: 1 })
        .await
        .unwrap();
    assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");

    // Um pedido absurdo (1 hora) fica limitado a MAX_WAIT_SECS: responde vazio a tempo.
    let t = Instant::now();
    let r = e
        .request_with_timeout(
            ClientMessage::WaitMessages { user_id: uid(&eve), timeout_secs: 3600 },
            Duration::from_secs(qt_server::waiters::MAX_WAIT_SECS + 10),
        )
        .await
        .unwrap();
    assert!(matches!(r, ServerMessage::Messages { ref messages } if messages.is_empty()), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(qt_server::waiters::MAX_WAIT_SECS + 3));
}

#[tokio::test]
async fn ninguem_a_espera_nao_deixa_lixo_em_memoria() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let bob = SigningKeyPair::generate();
    let mut b = client(&relay, &bob).await;
    assert!(b.wait_messages(1).await.unwrap().is_empty());
    assert!(!qt_server::waiters::is_waiting(&uid(&bob)), "a espera terminou: a entrada devia ter saido");

    // Duas esperas do mesmo utilizador: a entrada so sai quando a ultima acaba.
    let w1 = qt_server::waiters::Waiter::register(&uid(&bob));
    let w2 = qt_server::waiters::Waiter::register(&uid(&bob));
    drop(w1);
    assert!(qt_server::waiters::is_waiting(&uid(&bob)));
    drop(w2);
    assert!(!qt_server::waiters::is_waiting(&uid(&bob)));
}
