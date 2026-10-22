//! Denuncias (`SubmitReport`) e pedidos do operador (`Admin*`): so a chave do operador
//! lista, apaga e bane; uma conta banida deixa de entrar. Precisa de um Redis em 127.0.0.1:6379.

mod common;
use common::*;

use qt_core::primitives::SigningKeyPair;
use qt_core::report;
use qt_server::client::{RelayClient, RelayConfig, RelayError};
use qt_server::protocol::*;
use qt_server::ServerConfig;
use qt_transport::generate_static_keypair;

const SEED: [u8; 32] = [42u8; 32];

fn uid(s: &SigningKeyPair) -> String {
    user_id_from_signing_public(&s.verifying_key.to_bytes())
}

fn operator() -> SigningKeyPair {
    SigningKeyPair::from_bytes(&report::operator_signing_private(&SEED))
}

fn cfg() -> ServerConfig {
    ServerConfig { operator: Some(uid(&operator())), ..ServerConfig::default() }
}

async fn client(relay: &TestRelay, s: &SigningKeyPair) -> Result<RelayClient, RelayError> {
    let n = generate_static_keypair().unwrap();
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    RelayClient::connect(&relay.url, n.private, s.to_bytes(), cfg).await
}

fn sealed(text: &str) -> Vec<u8> {
    report::seal_with(&report::operator_report_public(&SEED), text.as_bytes()).unwrap()
}

async fn list(op: &mut RelayClient) -> Vec<StoredReport> {
    match op.request(ClientMessage::AdminListReports { after: 0 }).await.unwrap() {
        ServerMessage::Reports { reports, more } => {
            assert!(!more);
            reports
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn denuncia_guardada_cifrada_e_lida_so_pelo_operador() {
    let relay = TestRelay::start(cfg()).await;
    let alice = SigningKeyPair::generate();
    let mut a = client(&relay, &alice).await.unwrap();
    a.register_bundle(valid_bundle(&alice, 1)).await.unwrap();

    let r = a.request(ClientMessage::SubmitReport { sealed: sealed("spam do bob") }).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack), "{r:?}");

    // Quem nao e operador nao lista.
    let r = a.request(ClientMessage::AdminListReports { after: 0 }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == NOT_OPERATOR), "{r:?}");

    // O operador lista, abre com a semente e apaga.
    let mut op = client(&relay, &operator()).await.unwrap();
    let reports = list(&mut op).await;
    assert_eq!(reports.len(), 1);
    assert_eq!(report::open(&SEED, &reports[0].sealed).unwrap(), b"spam do bob");
    assert!(reports[0].received_at_ms > 0);
    let r = op.request(ClientMessage::AdminDeleteReport { id: reports[0].id }).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack));
    assert!(list(&mut op).await.is_empty());
}

#[tokio::test]
async fn denuncias_invalidas_sem_conta_ou_a_mais_sao_recusadas() {
    let relay = TestRelay::start(cfg()).await;
    let alice = SigningKeyPair::generate();
    let mut a = client(&relay, &alice).await.unwrap();

    // Sem bundle registado: recusada.
    let r = a.request(ClientMessage::SubmitReport { sealed: sealed("x") }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == NOT_REGISTERED), "{r:?}");

    a.register_bundle(valid_bundle(&alice, 2)).await.unwrap();
    for bad in [vec![], vec![1u8; 10], vec![9u8; 200], vec![1u8; report::MAX_SEALED + 1]] {
        let r = a.request(ClientMessage::SubmitReport { sealed: bad }).await.unwrap();
        assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
    }

    // Limite diario por conta.
    for _ in 0..qt_server::store::MAX_REPORTS_PER_DAY {
        let r = a.request(ClientMessage::SubmitReport { sealed: sealed("x") }).await.unwrap();
        assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    }
    let r = a.request(ClientMessage::SubmitReport { sealed: sealed("x") }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == REPORT_LIMIT), "{r:?}");
}

#[tokio::test]
async fn conta_banida_perde_tudo_e_deixa_de_entrar_ate_ser_desbanida() {
    let relay = TestRelay::start(cfg()).await;
    let (alice, bob) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut a = client(&relay, &alice).await.unwrap();
    let mut b = client(&relay, &bob).await.unwrap();
    a.register_bundle(valid_bundle(&alice, 3)).await.unwrap();
    b.register_bundle(valid_bundle(&bob, 4)).await.unwrap();
    a.send_message(&uid(&bob), vec![], b"ola".to_vec()).await.unwrap();

    // Outra conta nao bane.
    let r = a.request(ClientMessage::AdminBan { user_id: uid(&bob) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == NOT_OPERATOR), "{r:?}");

    let mut op = client(&relay, &operator()).await.unwrap();
    let r = op.request(ClientMessage::AdminBan { user_id: uid(&bob) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    match op.request(ClientMessage::AdminListBans).await.unwrap() {
        ServerMessage::Bans { user_ids } => assert_eq!(user_ids, vec![uid(&bob)]),
        other => panic!("{other:?}"),
    }

    // O bundle e a fila do Bob foram apagados; mensagens para ele sao recusadas.
    assert_eq!(a.fetch_bundle(&uid(&bob)).await.unwrap(), None);
    assert!(a.send_message(&uid(&bob), vec![], b"depois".to_vec()).await.is_err());
    // O Bob ja nao entra.
    assert!(matches!(client(&relay, &bob).await, Err(RelayError::AuthRejected)));

    // Desbanido: volta a entrar e a registar-se.
    let r = op.request(ClientMessage::AdminUnban { user_id: uid(&bob) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack));
    let mut b2 = client(&relay, &bob).await.unwrap();
    b2.register_bundle(valid_bundle(&bob, 5)).await.unwrap();
    drop(b);
}

#[tokio::test]
async fn sem_operador_configurado_ninguem_faz_pedidos_de_operador() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let mut op = client(&relay, &operator()).await.unwrap();
    let r = op.request(ClientMessage::AdminListBans).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == NOT_OPERATOR), "{r:?}");
}

#[tokio::test]
async fn premium_dado_pelo_operador_e_lido_pela_conta() {
    let relay = TestRelay::start(cfg()).await;
    let alice = SigningKeyPair::generate();
    let mut a = client(&relay, &alice).await.unwrap();
    let r = a.request(ClientMessage::PremiumStatus).await.unwrap();
    assert!(matches!(r, ServerMessage::Premium { until_ms: 0, .. }), "{r:?}");

    // Outra conta nao da premium.
    let r = a.request(ClientMessage::AdminGrantPremium { user_id: uid(&alice), until_ms: u64::MAX / 2 }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == NOT_OPERATOR), "{r:?}");

    let mut op = client(&relay, &operator()).await.unwrap();
    let until = 4_102_444_800_000; // 2100
    let r = op.request(ClientMessage::AdminGrantPremium { user_id: uid(&alice), until_ms: until }).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    let r = a.request(ClientMessage::PremiumStatus).await.unwrap();
    assert!(matches!(r, ServerMessage::Premium { until_ms, .. } if until_ms == until), "{r:?}");
    match op.request(ClientMessage::AdminListPremium).await.unwrap() {
        ServerMessage::PremiumList { entries } => assert_eq!(entries, vec![PremiumEntry { user_id: uid(&alice), until_ms: until }]),
        other => panic!("{other:?}"),
    }

    // Tirar.
    let r = op.request(ClientMessage::AdminGrantPremium { user_id: uid(&alice), until_ms: 0 }).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack));
    assert!(matches!(a.request(ClientMessage::PremiumStatus).await.unwrap(), ServerMessage::Premium { until_ms: 0, .. }));
    match op.request(ClientMessage::AdminListPremium).await.unwrap() {
        ServerMessage::PremiumList { entries } => assert!(entries.is_empty()),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn bloqueio_premium_recusa_anexos_e_chamadas_a_quem_nao_tem() {
    let dir = std::env::temp_dir().join(format!("qt_premium_blobs_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let relay = TestRelay::start_with_blobs(cfg(), qt_server::BlobConfig::new(dir)).await;
    let (alice, bob) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut a = client(&relay, &alice).await.unwrap();
    let mut b = client(&relay, &bob).await.unwrap();
    a.register_bundle(valid_bundle(&alice, 1)).await.unwrap();
    b.register_bundle(valid_bundle(&bob, 2)).await.unwrap();
    let call = |wake| ClientMessage::SendMessage {
        to: uid(&bob),
        sealed_from: vec![],
        ciphertext: b"oferta".to_vec(),
        silent: false,
        wake,
    };
    let refused = |r: &ServerMessage| matches!(r, ServerMessage::Error { reason } if reason == PREMIUM_REQUIRED);

    // Bloqueio desligado (por omissao): tudo passa.
    assert!(matches!(a.request(ClientMessage::BlobCreate { size: 10 }).await.unwrap(), ServerMessage::BlobCreated { .. }));
    assert!(matches!(a.request(call(Some(WakeKind::Call))).await.unwrap(), ServerMessage::Ack));
    assert!(matches!(a.request(ClientMessage::PremiumStatus).await.unwrap(), ServerMessage::Premium { enforced: false, .. }));

    // So o operador liga o bloqueio.
    let r = a.request(ClientMessage::AdminPremiumEnforce { on: Some(false) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == NOT_OPERATOR), "{r:?}");
    let mut op = client(&relay, &operator()).await.unwrap();
    let r = op.request(ClientMessage::AdminPremiumEnforce { on: Some(true) }).await.unwrap();
    assert!(matches!(r, ServerMessage::PremiumEnforce { on: true }), "{r:?}");
    let r = op.request(ClientMessage::AdminPremiumEnforce { on: None }).await.unwrap();
    assert!(matches!(r, ServerMessage::PremiumEnforce { on: true }), "{r:?}");

    // Sem premium: anexos e chamadas recusados; texto, fim de chamada e receber continuam.
    assert!(matches!(a.request(ClientMessage::PremiumStatus).await.unwrap(), ServerMessage::Premium { until_ms: 0, enforced: true }));
    assert!(refused(&a.request(ClientMessage::BlobCreate { size: 10 }).await.unwrap()));
    assert!(refused(&a.request(call(Some(WakeKind::Call))).await.unwrap()));
    assert!(matches!(a.request(call(None)).await.unwrap(), ServerMessage::Ack));
    assert!(matches!(a.request(call(Some(WakeKind::CallEnd))).await.unwrap(), ServerMessage::Ack));

    // Premium que ja acabou nao conta.
    op.request(ClientMessage::AdminGrantPremium { user_id: uid(&alice), until_ms: 1_000 }).await.unwrap();
    assert!(refused(&a.request(ClientMessage::BlobCreate { size: 10 }).await.unwrap()));

    // Com premium: passa.
    op.request(ClientMessage::AdminGrantPremium { user_id: uid(&alice), until_ms: 4_102_444_800_000 }).await.unwrap();
    assert!(matches!(a.request(ClientMessage::BlobCreate { size: 10 }).await.unwrap(), ServerMessage::BlobCreated { .. }));
    assert!(matches!(a.request(call(Some(WakeKind::Call))).await.unwrap(), ServerMessage::Ack));

    // Desligar o bloqueio: o Bob (sem premium) volta a enviar anexos.
    let r = op.request(ClientMessage::AdminPremiumEnforce { on: Some(false) }).await.unwrap();
    assert!(matches!(r, ServerMessage::PremiumEnforce { on: false }), "{r:?}");
    assert!(matches!(b.request(ClientMessage::BlobCreate { size: 10 }).await.unwrap(), ServerMessage::BlobCreated { .. }));
}
