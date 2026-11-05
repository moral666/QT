//! Limites por conta e por ligacao: contas novas, filas, copias, anexos, saida e pedidos sem
//! conta. Precisa de um Redis em 127.0.0.1:6379.
//!
//! O limite global de contas novas fica desligado em `cfg()`: e do processo inteiro e so o
//! teste dele o usa.

mod common;
use common::*;

use base64::Engine;
use qt_core::primitives::SigningKeyPair;
use qt_core::report;
use qt_server::blobs::{BlobError, BLOB_CHUNK, UNFINISHED_TTL};
use qt_server::client::{RelayClient, RelayConfig, RelayError};
use qt_server::limits::{DailyRate, Rate};
use qt_server::protocol::*;
use qt_server::store::{NewAccount, Sender, StoreError, AGE_MARK_SECONDS, BUNDLE_TTL_SECONDS, MAX_QUEUED_BYTES_PER_USER, NEW_ACCOUNT_OTPKS, NEW_BUNDLE_TTL_SECONDS, QUEUE_TTL_SECONDS};
use qt_server::{BlobConfig, ServerConfig, Store};
use qt_transport::generate_static_keypair;
use std::time::{Duration, Instant};

const OFF: Rate = Rate { burst: 0, per_minute: 0 };
const SEED: [u8; 32] = [42u8; 32];

fn cfg() -> ServerConfig {
    ServerConfig { require_pow: false, new_accounts: OFF, ..ServerConfig::default() }
}

fn uid(s: &SigningKeyPair) -> String {
    user_id_from_signing_public(&s.verifying_key.to_bytes())
}

async fn client(relay: &TestRelay, s: &SigningKeyPair) -> Result<RelayClient, RelayError> {
    let n = generate_static_keypair().unwrap();
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    RelayClient::connect(&relay.url, n.private, s.to_bytes(), cfg).await
}

fn store(relay: &TestRelay) -> Store {
    Store::with_prefix("redis://127.0.0.1:6379", &relay.prefix).unwrap()
}

async fn redis() -> redis::aio::MultiplexedConnection {
    redis::Client::open("redis://127.0.0.1:6379").unwrap().get_multiplexed_async_connection().await.unwrap()
}

async fn ttl(key: String) -> i64 {
    let mut c = redis().await;
    redis::cmd("TTL").arg(key).query_async(&mut c).await.unwrap()
}

async fn exists(key: String) -> bool {
    let mut c = redis().await;
    redis::cmd("EXISTS").arg(key).query_async(&mut c).await.unwrap()
}

async fn llen(key: String) -> i64 {
    let mut c = redis().await;
    redis::cmd("LLEN").arg(key).query_async(&mut c).await.unwrap()
}

fn register(s: &SigningKeyPair, pow: Option<u64>, keys: Option<Vec<Vec<u8>>>) -> ClientMessage {
    ClientMessage::RegisterPreKeyBundle { user_id: uid(s), bundle_bytes: valid_bundle(s, 1), pow, one_time_pre_keys: keys }
}

fn otpk(id: u32) -> Vec<u8> {
    let mut k = id.to_be_bytes().to_vec();
    k.extend_from_slice(&[id as u8; 32]);
    k
}

fn refused(r: ServerMessage, reason: &str) {
    assert!(matches!(&r, ServerMessage::Error { reason: x } if x == reason), "esperava \"{reason}\": {r:?}");
}

fn send_to(to: &str, size: usize) -> ClientMessage {
    ClientMessage::SendMessage { to: to.into(), sealed_from: vec![1; 8], ciphertext: vec![2; size], silent: false, wake: None }
}

fn blobs_cfg(tag: &str) -> BlobConfig {
    let dir = std::env::temp_dir().join(format!("qt-protecoes-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut c = BlobConfig::new(dir);
    c.min_free_bytes = 0;
    c
}

fn prefix(tag: &str) -> String {
    format!("prot-{tag}-{}", std::process::id())
}

/// Numero que nao e prova de trabalho valida para esta conta.
fn wrong_pow(s: &SigningKeyPair) -> u64 {
    let key = s.verifying_key.to_bytes();
    (0u64..).find(|n| !qt_core::pow::pow_verify(&key, *n, qt_core::pow::POW_BITS)).unwrap()
}

#[tokio::test]
async fn tres_provas_erradas_fecham_a_ligacao() {
    let relay = TestRelay::start(ServerConfig { new_accounts: OFF, ..ServerConfig::default() }).await;
    let s = SigningKeyPair::generate();
    let mut c = client(&relay, &s).await.unwrap();
    let nonce = wrong_pow(&s);
    for _ in 0..3 {
        refused(c.request(register(&s, Some(nonce), None)).await.unwrap(), POW_REQUIRED);
    }
    assert!(c.fetch_messages().await.is_err(), "a ligacao devia ter fechado");
    // Sem prova tambem conta: a app normal so pede uma vez.
    let mut c = client(&relay, &s).await.unwrap();
    refused(c.request(register(&s, None, None)).await.unwrap(), POW_REQUIRED);
    assert!(c.fetch_messages().await.is_ok());
}

#[tokio::test]
async fn registo_limitado_por_conta() {
    let relay = TestRelay::start(ServerConfig { register_rate: Rate { burst: 2, per_minute: 1 }, ..cfg() }).await;
    let s = SigningKeyPair::generate();
    let mut c = client(&relay, &s).await.unwrap();
    for _ in 0..2 {
        assert!(matches!(c.request(register(&s, None, None)).await.unwrap(), ServerMessage::Ack));
    }
    refused(c.request(register(&s, None, None)).await.unwrap(), RATE_LIMITED);
}

#[tokio::test]
async fn teto_de_contas_novas_por_relay() {
    let relay = TestRelay::start(ServerConfig { new_accounts: Rate { burst: 2, per_minute: 1 }, ..cfg() }).await;
    let mut contas = Vec::new();
    for _ in 0..2 {
        let s = SigningKeyPair::generate();
        let mut c = client(&relay, &s).await.unwrap();
        assert!(matches!(c.request(register(&s, None, None)).await.unwrap(), ServerMessage::Ack));
        contas.push((s, c));
    }
    let s = SigningKeyPair::generate();
    let mut c = client(&relay, &s).await.unwrap();
    refused(c.request(register(&s, None, None)).await.unwrap(), NO_NEW_ACCOUNTS);
    // Quem ja tem conta continua a renovar o registo.
    let (s, mut c) = contas.pop().unwrap();
    assert!(matches!(c.request(register(&s, None, None)).await.unwrap(), ServerMessage::Ack));
}

#[tokio::test]
async fn sem_conta_so_o_necessario_para_abrir_a_app() {
    let operator = SigningKeyPair::from_bytes(&report::operator_signing_private(&SEED));
    let relay = TestRelay::start(ServerConfig { operator: Some(uid(&operator)), ..cfg() }).await;
    let s = SigningKeyPair::generate();
    let mut c = client(&relay, &s).await.unwrap();
    let id = "a".repeat(32);
    for m in [
        ClientMessage::FetchPreKeyBundle { user_id: uid(&operator) },
        ClientMessage::BlobGet { blob_id: id.clone(), offset: 0 },
        ClientMessage::BlobInfo { blob_id: id.clone() },
        ClientMessage::BlobDelete { blob_id: id.clone() },
        ClientMessage::BlobPut { blob_id: id.clone(), token: "t".into(), offset: 0, data: "AA==".into() },
        ClientMessage::CommunityVersion,
        ClientMessage::CommunityIndex,
        ClientMessage::CommunityPhotos { hashes: vec![] },
    ] {
        refused(c.request(m).await.unwrap(), NOT_REGISTERED);
    }
    // Ao abrir, a app le a fila, o premium e, numa recuperacao, a copia.
    assert!(c.fetch_messages().await.is_ok());
    assert!(matches!(c.request(ClientMessage::PremiumStatus).await.unwrap(), ServerMessage::Premium { .. }));
    let r = c.request(ClientMessage::BackupGet { lookup: "b".repeat(64), part: 0 }).await.unwrap();
    assert!(matches!(r, ServerMessage::BackupNotFound), "{r:?}");
    // Depois de registar, tudo funciona na mesma ligacao.
    assert!(matches!(c.request(register(&s, None, None)).await.unwrap(), ServerMessage::Ack));
    assert!(matches!(c.request(ClientMessage::CommunityVersion).await.unwrap(), ServerMessage::CommunityVersion { .. }));
    // O operador nao tem conta nem precisa.
    let mut op = client(&relay, &operator).await.unwrap();
    assert!(matches!(op.request(ClientMessage::CommunityVersion).await.unwrap(), ServerMessage::CommunityVersion { .. }));
}

#[tokio::test]
async fn ligacao_sem_conta_tem_tempo_e_pedidos_contados() {
    let relay = TestRelay::start(ServerConfig { unregistered_life: Duration::from_millis(300), ..cfg() }).await;
    let (s, r) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut unreg = client(&relay, &s).await.unwrap();
    let mut reg = client(&relay, &r).await.unwrap();
    assert!(matches!(reg.request(register(&r, None, None)).await.unwrap(), ServerMessage::Ack));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(unreg.fetch_messages().await.is_err(), "sem conta: tempo maximo");
    assert!(reg.fetch_messages().await.is_ok(), "com conta: fica");

    let relay = TestRelay::start(ServerConfig { unregistered_requests: 3, ..cfg() }).await;
    let mut unreg = client(&relay, &SigningKeyPair::generate()).await.unwrap();
    for _ in 0..3 {
        assert!(unreg.fetch_messages().await.is_ok());
    }
    assert!(unreg.fetch_messages().await.is_err(), "sem conta: pedidos contados");

    // Com 0 nao ha limites para estas ligacoes (medicoes).
    let cfg = ServerConfig { max_unregistered: 0, unregistered_life: Duration::from_millis(100), unregistered_requests: 1, ..cfg() };
    let relay = TestRelay::start(cfg).await;
    let mut unreg = client(&relay, &SigningKeyPair::generate()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    for _ in 0..3 {
        assert!(unreg.fetch_messages().await.is_ok());
    }
}

#[tokio::test]
async fn primeiras_24_horas_da_conta() {
    let relay = TestRelay::start(ServerConfig { send_rate_new: Rate { burst: 2, per_minute: 1 }, ..cfg() }).await;
    let s = SigningKeyPair::generate();
    let me = uid(&s);
    let p = relay.prefix.clone();
    let mut c = client(&relay, &s).await.unwrap();
    let keys: Vec<Vec<u8>> = (1..=50).map(otpk).collect();
    assert!(matches!(c.request(register(&s, None, Some(keys))).await.unwrap(), ServerMessage::Ack));
    assert_eq!(llen(format!("{p}:otpk:{me}")).await, NEW_ACCOUNT_OTPKS as i64);
    let days = ttl(format!("{p}:bundle:{me}")).await;
    assert!(days > 0 && days <= NEW_BUNDLE_TTL_SECONDS, "{days}");

    // Mensagens por minuto mais baixas nas primeiras 24 horas.
    let friend = SigningKeyPair::generate();
    registar(&relay, &uid(&friend)).await;
    for _ in 0..2 {
        assert!(matches!(c.request(send_to(&uid(&friend), 100)).await.unwrap(), ServerMessage::Ack));
    }
    refused(c.request(send_to(&uid(&friend), 100)).await.unwrap(), RATE_LIMITED);

    // Registar outra vez dentro das 24 horas nao estende os prazos.
    let keys: Vec<Vec<u8>> = (1..=50).map(otpk).collect();
    assert!(matches!(c.request(register(&s, None, Some(keys))).await.unwrap(), ServerMessage::Ack));
    assert_eq!(llen(format!("{p}:otpk:{me}")).await, NEW_ACCOUNT_OTPKS as i64);
    assert!(ttl(format!("{p}:bundle:{me}")).await <= NEW_BUNDLE_TTL_SECONDS);

    // Passadas as 24 horas, a app volta a ligar-se e o bundle fica com o prazo normal.
    let mut r = redis().await;
    let _: () = redis::cmd("EXPIRE").arg(format!("{p}:idade:{me}")).arg(AGE_MARK_SECONDS - 25 * 3600).query_async(&mut r).await.unwrap();
    drop(c);
    let mut c = client(&relay, &s).await.unwrap();
    assert!(ttl(format!("{p}:bundle:{me}")).await > NEW_BUNDLE_TTL_SECONDS);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(matches!(c.request(send_to(&uid(&friend), 100)).await.unwrap(), ServerMessage::Ack));
}

#[tokio::test]
async fn bytes_por_dia_por_conta() {
    let relay = TestRelay::start(ServerConfig { send_bytes: DailyRate { per_day: 2_500 }, ..cfg() }).await;
    let (s, friend) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    registar(&relay, &uid(&s)).await;
    registar(&relay, &uid(&friend)).await;
    let mut c = client(&relay, &s).await.unwrap();
    for _ in 0..2 {
        assert!(matches!(c.request(send_to(&uid(&friend), 1_000)).await.unwrap(), ServerMessage::Ack));
    }
    refused(c.request(send_to(&uid(&friend), 1_000)).await.unwrap(), RATE_LIMITED);
}

#[tokio::test]
async fn fila_limitada_em_bytes_com_parte_final_para_contas_antigas() {
    let s = Store::with_prefix("redis://127.0.0.1:6379", &prefix("fila")).unwrap();
    let (to, from) = ("c".repeat(64), "d".repeat(64));
    s.delete_account(&to).await.unwrap();
    s.register_bundle(from.clone(), vec![1]).await.unwrap();
    s.register_bundle(to.clone(), vec![1]).await.unwrap();
    let size = 14_000usize;
    let mut sent = 0i64;
    // Conta nova: para aos 80 %.
    loop {
        match s.enqueue_seats(Sender::Registered, to.clone(), vec![], vec![7; size], true).await {
            Ok(_) => sent += size as i64,
            Err(StoreError::QueueFull) => break,
            Err(e) => panic!("{e:?}"),
        }
    }
    assert!(sent <= MAX_QUEUED_BYTES_PER_USER * 80 / 100 + size as i64, "{sent}");
    assert!(sent >= MAX_QUEUED_BYTES_PER_USER * 70 / 100, "{sent}");
    // Conta antiga: usa o resto, ate ao limite.
    loop {
        match s.enqueue_seats(Sender::Registered, to.clone(), vec![], vec![7; size], false).await {
            Ok(_) => sent += size as i64,
            Err(StoreError::QueueFull) => break,
            Err(e) => panic!("{e:?}"),
        }
    }
    assert!(sent > MAX_QUEUED_BYTES_PER_USER * 90 / 100 && sent <= MAX_QUEUED_BYTES_PER_USER, "{sent}");
    // Confirmar liberta lugar.
    let page = s.peek_messages(&to, 40_000).await.unwrap();
    let ids: Vec<u64> = page.iter().map(|m| m.id).collect();
    assert_eq!(s.ack_messages(&to, &ids).await.unwrap(), ids.len());
    s.enqueue(Sender::Registered, to.clone(), vec![], vec![7; size]).await.unwrap();
    s.delete_account(&to).await.unwrap();
}

#[tokio::test]
async fn pedido_recusado_nao_deixa_nada_no_redis() {
    let p = prefix("recusa");
    let s = Store::with_prefix("redis://127.0.0.1:6379", &p).unwrap();
    let to = "e".repeat(64);
    let r = s.enqueue(Sender::Registered, to.clone(), vec![], vec![1; 10]).await;
    assert!(matches!(r, Err(StoreError::RecipientUnknown)), "{r:?}");
    assert!(!exists(format!("{p}:queue:{to}")).await);
    assert!(!exists(format!("{p}:qbytes:{to}")).await);
}

#[tokio::test]
async fn fila_de_quem_nunca_leu_dura_menos() {
    let p = prefix("nunca");
    let s = Store::with_prefix("redis://127.0.0.1:6379", &p).unwrap();
    let to = "f".repeat(64);
    s.delete_account(&to).await.unwrap();
    s.register_full(&to, vec![1], None, NewAccount::Created).await.unwrap();
    s.enqueue(Sender::Registered, to.clone(), vec![], vec![1; 10]).await.unwrap();
    assert!(ttl(format!("{p}:queue:{to}")).await <= 7 * 86_400);
    s.mark_read(&to).await.unwrap();
    assert!(ttl(format!("{p}:bundle:{to}")).await > NEW_BUNDLE_TTL_SECONDS, "conta em uso");
    assert!(ttl(format!("{p}:bundle:{to}")).await <= BUNDLE_TTL_SECONDS as i64);
    s.enqueue(Sender::Registered, to.clone(), vec![], vec![1; 10]).await.unwrap();
    assert!(ttl(format!("{p}:queue:{to}")).await > 7 * 86_400 && ttl(format!("{p}:queue:{to}")).await <= QUEUE_TTL_SECONDS);
    s.delete_account(&to).await.unwrap();
    assert!(!exists(format!("{p}:idade:{to}")).await);
    assert!(!exists(format!("{p}:nunca:{to}")).await);
}

#[tokio::test]
async fn uma_copia_a_meio_de_cada_vez_por_conta() {
    let p = prefix("copia");
    let s = Store::with_prefix("redis://127.0.0.1:6379", &p).unwrap();
    let owner = "1".repeat(64);
    let lookup = "2".repeat(64);
    let (g1, g2, g3) = ("a".repeat(16), "b".repeat(16), "c".repeat(16));
    s.delete_backup(&owner).await.unwrap();
    s.backup_put(&owner, &lookup, &g1, 0, 2, vec![1; 100]).await.unwrap();
    assert!(exists(format!("{p}:bkpart:{lookup}:{g1}:0")).await, "chave da parte");
    // A segunda geracao leva a primeira.
    s.backup_put(&owner, &lookup, &g2, 0, 2, vec![2; 100]).await.unwrap();
    assert!(!exists(format!("{p}:bkpart:{lookup}:{g1}:0")).await);
    s.backup_put(&owner, &lookup, &g2, 1, 2, vec![2; 100]).await.unwrap();
    assert_eq!(s.backup_get(&lookup, 0).await.unwrap().unwrap().0, 2);
    // Uma copia nova que fique a meio nao toca na publicada.
    s.backup_put(&owner, &lookup, &g3, 0, 2, vec![3; 100]).await.unwrap();
    assert_eq!(s.backup_get(&lookup, 1).await.unwrap().unwrap().1, vec![2; 100]);
    s.delete_backup(&owner).await.unwrap();
}

#[tokio::test]
async fn banir_guarda_a_copia_durante_30_dias() {
    let p = prefix("ban");
    let s = Store::with_prefix("redis://127.0.0.1:6379", &p).unwrap();
    let owner = "3".repeat(64);
    let lookup = "4".repeat(64);
    let g = "a".repeat(16);
    s.delete_backup(&owner).await.unwrap();
    s.backup_put(&owner, &lookup, &g, 0, 1, vec![1; 100]).await.unwrap();
    s.ban(&owner).await.unwrap();
    s.expire_backup_of_banned(&owner).await.unwrap();
    assert_eq!(s.backup_get(&lookup, 0).await.unwrap().unwrap().1, vec![1; 100]);
    let left = ttl(format!("{p}:bkpart:{lookup}:{g}:0")).await;
    assert!(left > 0 && left <= 30 * 86_400, "{left}");
    // Levantado o banimento, a copia volta ao prazo normal quando a app abre.
    s.unban(&owner).await.unwrap();
    s.renew_backup_of(&owner, &lookup).await.unwrap();
    assert!(ttl(format!("{p}:bkpart:{lookup}:{g}:0")).await > 30 * 86_400);
    s.delete_backup(&owner).await.unwrap();
}

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn blob_store(tag: &str, f: impl FnOnce(&mut BlobConfig)) -> Store {
    let mut s = Store::with_prefix("redis://127.0.0.1:6379", &format!("{}-{}", prefix(tag), std::process::id())).unwrap();
    let mut c = blobs_cfg(tag);
    f(&mut c);
    s.enable_blobs(c).unwrap();
    s
}

#[tokio::test]
async fn anexos_sem_premium_tem_quota_e_espaco_proprios() {
    let s = blob_store("quota", |c| {
        c.max_free_user_bytes_per_day = 1_000;
        c.max_total_bytes = 100_000;
    });
    let user = format!("{:064x}", rand_id());
    s.blob_create_as(&user, 600, true).await.unwrap();
    assert!(matches!(s.blob_create_as(&user, 600, true).await, Err(BlobError::Quota)));
    // Quem paga nao tem esta quota.
    s.blob_create_as(&format!("{:064x}", rand_id()), 5_000, false).await.unwrap();

    // So 25 % do espaco para quem nao tem premium.
    let s = blob_store("parte", |c| c.max_total_bytes = 10_000);
    let user = format!("{:064x}", rand_id());
    assert!(matches!(s.blob_create_as(&user, 3_000, true).await, Err(BlobError::NoSpace)));
    s.blob_create_as(&user, 2_000, true).await.unwrap();
    s.blob_create_as(&format!("{:064x}", rand_id()), 7_000, false).await.unwrap();
}

fn rand_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64;
    t ^ N.fetch_add(0x9E37_79B9, Ordering::Relaxed)
}

#[tokio::test]
async fn envio_parado_perde_o_lugar_e_o_anexo_acabado_fica_24_horas() {
    let tag = "prazos";
    let p = format!("{}-{}", prefix(tag), std::process::id());
    let s = blob_store(tag, |_| {});
    let (id, token) = s.blob_create(&format!("{:064x}", rand_id()), 10).await.unwrap();
    let left = ttl(format!("{p}:blob:{id}")).await;
    assert!(left > 0 && left <= UNFINISHED_TTL.as_secs() as i64, "{left}");
    s.blob_put(&id, &token, 0, &b64(&[9u8; 10])).await.unwrap();
    assert!(ttl(format!("{p}:blob:{id}")).await > UNFINISHED_TTL.as_secs() as i64);
}

#[tokio::test]
async fn descargas_limitadas_por_anexo() {
    let s = blob_store("descargas", |_| {});
    let (id, token) = s.blob_create(&format!("{:064x}", rand_id()), 100).await.unwrap();
    s.blob_put(&id, &token, 0, &b64(&[9u8; 100])).await.unwrap();
    for _ in 0..qt_server::blobs::MAX_DOWNLOADS {
        s.blob_get(&id, 0).await.unwrap();
    }
    assert!(matches!(s.blob_get(&id, 0).await, Err(BlobError::NotFound)));
}

#[tokio::test]
async fn saida_com_ritmo_por_ligacao_e_limite_por_dia() {
    let blobs = blobs_cfg("saida");
    let relay = TestRelay::start_with_blobs(ServerConfig { egress_per_sec: 200_000, ..cfg() }, blobs).await;
    let s = SigningKeyPair::generate();
    registar(&relay, &uid(&s)).await;
    let mut c = client(&relay, &s).await.unwrap();
    let r = c.request(ClientMessage::BlobCreate { size: 2 * BLOB_CHUNK as u64 }).await.unwrap();
    let ServerMessage::BlobCreated { blob_id, token } = r else { panic!("{r:?}") };
    let data = base64::engine::general_purpose::STANDARD.encode(vec![5u8; BLOB_CHUNK]);
    for offset in [0, BLOB_CHUNK as u64] {
        let m = ClientMessage::BlobPut { blob_id: blob_id.clone(), token: token.clone(), offset, data: data.clone() };
        assert!(matches!(c.request(m).await.unwrap(), ServerMessage::BlobStatus { .. }));
    }
    let t = Instant::now();
    for _ in 0..10 {
        let m = ClientMessage::BlobGet { blob_id: blob_id.clone(), offset: 0 };
        assert!(matches!(c.request(m).await.unwrap(), ServerMessage::BlobData { .. }));
    }
    assert!(t.elapsed() >= Duration::from_millis(1_500), "{:?}", t.elapsed());

    let blobs = blobs_cfg("saida-dia");
    let relay = TestRelay::start_with_blobs(ServerConfig { egress_bytes: DailyRate { per_day: 1_000 }, ..cfg() }, blobs).await;
    let s = SigningKeyPair::generate();
    registar(&relay, &uid(&s)).await;
    let mut c = client(&relay, &s).await.unwrap();
    let r = c.request(ClientMessage::BlobCreate { size: 100 }).await.unwrap();
    let ServerMessage::BlobCreated { blob_id, token } = r else { panic!("{r:?}") };
    let data = base64::engine::general_purpose::STANDARD.encode(vec![5u8; 100]);
    let m = ClientMessage::BlobPut { blob_id: blob_id.clone(), token, offset: 0, data };
    assert!(matches!(c.request(m).await.unwrap(), ServerMessage::BlobStatus { .. }));
    let mut limited = false;
    for _ in 0..20 {
        let m = ClientMessage::BlobGet { blob_id: blob_id.clone(), offset: 0 };
        if matches!(c.request(m).await.unwrap(), ServerMessage::Error { .. }) {
            limited = true;
            break;
        }
    }
    assert!(limited, "o limite diario devia ter parado as descargas");
    assert!(c.fetch_messages().await.is_ok(), "as mensagens continuam");
}

#[tokio::test]
async fn one_time_prekeys_limitadas_por_alvo() {
    let relay = TestRelay::start(ServerConfig { otpks_per_hour: 2, ..cfg() }).await;
    let (bob, alice) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut b = client(&relay, &bob).await.unwrap();
    let keys = (1..=5).map(otpk).collect();
    assert!(matches!(b.request(register(&bob, None, Some(keys))).await.unwrap(), ServerMessage::Ack));
    registar(&relay, &uid(&alice)).await;
    let mut a = client(&relay, &alice).await.unwrap();
    let mut got = Vec::new();
    for _ in 0..3 {
        match a.request(ClientMessage::FetchPreKeyBundle { user_id: uid(&bob) }).await.unwrap() {
            ServerMessage::PreKeyBundle { one_time_pre_key, .. } => got.push(one_time_pre_key.is_some()),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(got, vec![true, true, false], "o bundle continua a vir, sem prekey");
}

#[tokio::test]
async fn contas_com_menos_de_7_dias_nao_ocupam_denuncias_nem_comunidade() {
    let relay = TestRelay::start(cfg()).await;
    let s = SigningKeyPair::generate();
    let mut c = client(&relay, &s).await.unwrap();
    assert!(matches!(c.request(register(&s, None, None)).await.unwrap(), ServerMessage::Ack));
    let sealed = report::seal_with(&report::operator_report_public(&SEED), b"x").unwrap();
    assert!(matches!(c.request(ClientMessage::SubmitReport { sealed }).await.unwrap(), ServerMessage::Ack));
    assert_eq!(llen(format!("{}:reports", relay.prefix)).await, 0);
    assert_eq!(store(&relay).reports_stored().await.unwrap(), 0);
}

#[tokio::test]
async fn estado_do_redis_para_as_estatisticas() {
    let s = Store::with_prefix("redis://127.0.0.1:6379", &prefix("info")).unwrap();
    assert!(s.redis_memory_pct().await.is_ok());
}
