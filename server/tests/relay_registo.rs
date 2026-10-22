//! Regressao (22/10/2026): so contas registadas (com bundle, logo com prova de
//! trabalho) escrevem no relay. Antes, uma chave inventada no momento conseguia
//! encher filas de IDs que nunca existiram, guardar tokens de 4 KB e criar anexos,
//! ate o Redis (512 MB, `noeviction`) parar o relay para toda a gente.
//! Precisa de um Redis em 127.0.0.1:6379.

mod common;
use common::*;

use qt_core::primitives::SigningKeyPair;
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::protocol::*;
use qt_server::{BlobConfig, Store};
use std::time::Duration;

fn cfg() -> RelayConfig {
    RelayConfig {
        connect_timeout: Duration::from_secs(3),
        request_timeout: Duration::from_secs(5),
        max_retries: 0,
        backoff_base: Duration::from_millis(50),
        expected_server_static: None,
    }
}

async fn ligar(relay: &TestRelay, s: &SigningKeyPair) -> RelayClient {
    let n = qt_transport::generate_static_keypair().unwrap();
    RelayClient::connect(&relay.url, n.private, s.to_bytes(), cfg()).await.unwrap()
}

/// Conta registada como as apps: bundle valido com prova de trabalho.
async fn registada(relay: &TestRelay) -> (SigningKeyPair, RelayClient) {
    let s = SigningKeyPair::generate();
    let mut c = ligar(relay, &s).await;
    c.register_bundle(valid_bundle(&s, 1)).await.unwrap();
    (s, c)
}

fn uid(s: &SigningKeyPair) -> String {
    user_id_from_signing_public(&s.verifying_key.to_bytes())
}

fn envio(to: &str) -> ClientMessage {
    ClientMessage::SendMessage { to: to.into(), sealed_from: vec![0; 64], ciphertext: vec![9; 1000], silent: true, wake: None }
}

fn recusa(r: ServerMessage, motivo: &str) {
    match r {
        ServerMessage::Error { reason } => assert_eq!(reason, motivo),
        other => panic!("esperava Error({motivo}), veio {other:?}"),
    }
}

fn store_de(relay: &TestRelay) -> Store {
    Store::with_prefix("redis://127.0.0.1:6379", &relay.prefix).unwrap()
}

#[tokio::test]
async fn chave_descartavel_nao_envia_mensagens() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (_, mut bob) = registada(&relay).await;
    let bob_id = bob.user_id().to_string();
    // Chave nova, sem bundle nem prova de trabalho: autentica-se, mas nao escreve.
    let mut lixo = ligar(&relay, &SigningKeyPair::generate()).await;
    recusa(lixo.request(envio(&bob_id)).await.unwrap(), NOT_REGISTERED);
    recusa(lixo.request(envio(&uid(&SigningKeyPair::generate()))).await.unwrap(), NOT_REGISTERED);
    assert!(bob.fetch_messages().await.unwrap().is_empty(), "nada ficou na fila do Bob");
}

#[tokio::test]
async fn conta_registada_nao_enche_filas_de_ids_inventados() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (_, mut alice) = registada(&relay).await;
    let inventado = uid(&SigningKeyPair::generate());
    for _ in 0..5 {
        recusa(alice.request(envio(&inventado)).await.unwrap(), RECIPIENT_UNKNOWN);
    }
    assert!(store_de(&relay).peek_messages(&inventado, 40_000).await.unwrap().is_empty(), "nenhuma fila criada");
}

#[tokio::test]
async fn contas_registadas_continuam_a_conversar() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (_, mut alice) = registada(&relay).await;
    let (_, mut bob) = registada(&relay).await;
    let bob_id = bob.user_id().to_string();
    alice.send_message(&bob_id, vec![1], b"ola".to_vec()).await.unwrap();
    let got = bob.fetch_messages().await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].ciphertext, b"ola");
    // Quem apagou a conta continua a dar "apagou a conta" (a app avisa), nao "desconhecido".
    bob.delete_account().await.unwrap();
    recusa(alice.request(envio(&bob_id)).await.unwrap(), RECIPIENT_DELETED);
}

#[tokio::test]
async fn token_de_notificacoes_so_com_conta_registada() {
    let relay = TestRelay::start(fast_cfg()).await;
    let s = SigningKeyPair::generate();
    let mut c = ligar(&relay, &s).await;
    let token = |t: &str| ClientMessage::RegisterPushToken { user_id: uid(&s), token: t.into() };
    recusa(c.request(token(&"x".repeat(4000))).await.unwrap(), NOT_REGISTERED);
    assert_eq!(store_de(&relay).get_push_token(&uid(&s)).await.unwrap(), None, "nada guardado");
    // Desligar as notificacoes (token vazio) e sempre aceite: so apaga.
    assert!(matches!(c.request(token("")).await.unwrap(), ServerMessage::Ack));
    // Depois de registar, o token e aceite.
    c.register_bundle(valid_bundle(&s, 2)).await.unwrap();
    assert!(matches!(c.request(token("abc:def")).await.unwrap(), ServerMessage::Ack));
}

#[tokio::test]
async fn anexo_novo_so_com_conta_registada() {
    let dir = std::env::temp_dir().join(format!("qt-registo-anexos-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut bc = BlobConfig::new(&dir);
    bc.min_free_bytes = 0;
    let relay = TestRelay::start_with_blobs(fast_cfg(), bc).await;
    let s = SigningKeyPair::generate();
    let mut c = ligar(&relay, &s).await;
    recusa(c.request(ClientMessage::BlobCreate { size: 1000 }).await.unwrap(), NOT_REGISTERED);
    c.register_bundle(valid_bundle(&s, 3)).await.unwrap();
    assert!(matches!(c.request(ClientMessage::BlobCreate { size: 1000 }).await.unwrap(), ServerMessage::BlobCreated { .. }));
    let _ = std::fs::remove_dir_all(dir);
}
