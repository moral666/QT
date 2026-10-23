//! Fichas anonimas do video pelo relay (0006), de ponta a ponta nos servidores:
//!   * EMISSAO no relay das mensagens real (conta autenticada, Redis): nunca passa do teto
//!     por conta e por mes, nem com pedidos ao mesmo tempo; so contas registadas; o operador
//!     muda o teto de uma conta; o Redis so guarda a contagem.
//!   * RESGATE no relay das chamadas real (UDP, sala anonima, registo das gastas no Redis):
//!     aceite uma vez, repetida recusada.
//!   * O RELAY NAO LIGA RESGATE A EMISSAO: o que o relay das chamadas recebe e guarda nao tem
//!     nada da conta nem nada do que o emissor viu.
//!
//! Precisa de um Redis em 127.0.0.1:6379.

mod common;
use common::*;

use base64::Engine;
use qt_core::call::{join_payload, parse_join_ok, Role, RoomKeys, JOIN_KIND, JOIN_PROLOGUE};
use qt_core::fichas::{self, Ficha, IssueRequest, Issuer, RedeemStatus, REDEEM_KIND, REDEEM_PROLOGUE};
use qt_core::primitives::SigningKeyPair;
use qt_server::call_relay::{serve, CallLimits, FichasSetup};
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::fichas::{IssuerKeys, SpentStore};
use qt_server::protocol::*;
use qt_server::ServerConfig;
use redis::AsyncCommands;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;

const SEED: [u8; 32] = [0x5e; 32];

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn uid(s: &SigningKeyPair) -> String {
    user_id_from_signing_public(&s.verifying_key.to_bytes())
}

fn operator() -> SigningKeyPair {
    SigningKeyPair::from_bytes(&[0x0b; 32])
}

fn cfg(per_month: u32) -> ServerConfig {
    ServerConfig {
        fichas: Some(Arc::new(IssuerKeys::new(SEED))),
        fichas_per_month: per_month,
        fichas_rate: qt_server::limits::Rate { burst: 0, per_minute: 0 },
        operator: Some(uid(&operator())),
        ..fast_cfg()
    }
}

async fn client(relay: &TestRelay, s: &SigningKeyPair) -> RelayClient {
    let n = qt_transport::generate_static_keypair().unwrap();
    RelayClient::connect(&relay.url, n.private, s.to_bytes(), RelayConfig { max_retries: 0, ..RelayConfig::default() }).await.unwrap()
}

struct Answer {
    epoch: u32,
    public_key: Vec<u8>,
    evaluated: Vec<u8>,
    proof: Vec<u8>,
    granted: u32,
    remaining: u32,
    per_month: u32,
}

async fn ask(c: &mut RelayClient, epoch: u32, blinded: &[u8]) -> Result<Answer, String> {
    match c.request(ClientMessage::IssueFichas { epoch, blinded: b64().encode(blinded) }).await.unwrap() {
        ServerMessage::Fichas { epoch, public_key, evaluated, proof, granted, remaining, per_month } => Ok(Answer {
            epoch,
            public_key: b64().decode(public_key).unwrap(),
            evaluated: b64().decode(evaluated).unwrap(),
            proof: b64().decode(proof).unwrap(),
            granted,
            remaining,
            per_month,
        }),
        ServerMessage::Error { reason } => Err(reason),
        other => panic!("{other:?}"),
    }
}

/// Pede `n` fichas e devolve as que vieram (com a prova verificada) e a resposta.
async fn issue(c: &mut RelayClient, n: usize) -> (Vec<Ficha>, Answer, Vec<u8>) {
    let epoch = fichas::current_epoch();
    let (req, blinded) = IssueRequest::new(epoch, n).unwrap();
    let a = ask(c, epoch, &blinded).await.unwrap();
    assert_eq!(a.epoch, epoch);
    let f = req.finalize(&a.public_key, &a.evaluated, &a.proof).expect("prova DLEQ valida");
    assert_eq!(f.len(), a.granted as usize);
    (f, a, blinded)
}

#[tokio::test]
async fn emissao_nunca_passa_do_teto_por_conta_e_por_mes() {
    let relay = TestRelay::start(cfg(10)).await;
    let alice = SigningKeyPair::generate();
    registar(&relay, &uid(&alice)).await;
    let mut a = client(&relay, &alice).await;

    let (f, ans, _) = issue(&mut a, 6).await;
    assert_eq!((f.len(), ans.remaining, ans.per_month), (6, 4, 10));
    assert_eq!(ans.public_key, Issuer::derive(&SEED, fichas::current_epoch()).unwrap().public_key().to_vec(), "chave do mes");
    // Pede mais 6: so vem o que falta ate ao teto (4), com prova para essas.
    let (f2, ans, _) = issue(&mut a, 6).await;
    assert_eq!((f2.len(), ans.remaining), (4, 0));
    // Teto atingido: nada mais este mes.
    let (f3, ans, _) = issue(&mut a, 3).await;
    assert_eq!((f3.len(), ans.granted, ans.remaining), (0, 0, 0));
    // Todas diferentes e todas validas para a chave do mes.
    let iss = Issuer::derive(&SEED, fichas::current_epoch()).unwrap();
    let mut seen = std::collections::HashSet::new();
    for x in f.iter().chain(&f2) {
        assert!(iss.verify(x.epoch, &x.nonce, &x.output));
        assert!(seen.insert(x.nonce));
    }

    // Outro mes (relogio do telemovel errado): nada dado, e diz qual e o mes do relay.
    let (_, blinded) = IssueRequest::new(190001, 2).unwrap();
    let ans = ask(&mut a, 190001, &blinded).await.unwrap();
    assert_eq!((ans.epoch, ans.granted), (fichas::current_epoch(), 0));

    // O operador muda a quota desta conta: +2.
    let mut op = client(&relay, &operator()).await;
    let r = op.request(ClientMessage::AdminSetFichasQuota { user_id: uid(&alice), per_month: Some(12) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    let (f4, ans, _) = issue(&mut a, 5).await;
    assert_eq!((f4.len(), ans.per_month, ans.remaining), (2, 12, 0));
    // Quem nao e operador nao muda tetos.
    let r = a.request(ClientMessage::AdminSetFichasQuota { user_id: uid(&alice), per_month: Some(999) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == NOT_OPERATOR), "{r:?}");

    // Conta sem registo (chave descartavel para juntar fichas): recusada.
    let bob = SigningKeyPair::generate();
    let mut b = client(&relay, &bob).await;
    let (_, blinded) = IssueRequest::new(fichas::current_epoch(), 2).unwrap();
    assert_eq!(ask(&mut b, fichas::current_epoch(), &blinded).await.err().as_deref(), Some(NOT_REGISTERED));

    // Pedidos invalidos: nada e gasto do teto.
    let carol = SigningKeyPair::generate();
    registar(&relay, &uid(&carol)).await;
    let mut c = client(&relay, &carol).await;
    let e = fichas::current_epoch();
    assert!(ask(&mut c, e, &[0u8; 31]).await.is_err(), "tamanho errado");
    assert!(ask(&mut c, e, &[0u8; 64]).await.is_err(), "identidade do grupo");
    assert!(ask(&mut c, e, &vec![0u8; (fichas::MAX_BATCH + 1) * 32]).await.is_err(), "lote grande demais");
    let (f, ans, _) = issue(&mut c, 10).await;
    assert_eq!((f.len(), ans.remaining), (10, 0), "os pedidos invalidos nao gastaram nada");
}

#[tokio::test]
async fn pedidos_ao_mesmo_tempo_nao_passam_do_teto() {
    let relay = TestRelay::start(cfg(10)).await;
    let dave = SigningKeyPair::generate();
    registar(&relay, &uid(&dave)).await;
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let (url, key) = (relay.url.clone(), dave.to_bytes());
        tasks.push(tokio::spawn(async move {
            let n = qt_transport::generate_static_keypair().unwrap();
            let mut c = RelayClient::connect(&url, n.private, key, RelayConfig { max_retries: 0, ..RelayConfig::default() }).await.unwrap();
            issue(&mut c, 4).await.0.len()
        }));
    }
    let mut total = 0;
    for t in tasks {
        total += t.await.unwrap();
    }
    assert_eq!(total, 10, "4 pedidos de 4 ao mesmo tempo, teto 10");
}

#[tokio::test]
async fn relay_sem_semente_nao_emite() {
    let relay = TestRelay::start(ServerConfig { fichas: None, ..fast_cfg() }).await;
    let eve = SigningKeyPair::generate();
    registar(&relay, &uid(&eve)).await;
    let mut c = client(&relay, &eve).await;
    let (_, blinded) = IssueRequest::new(fichas::current_epoch(), 1).unwrap();
    assert_eq!(ask(&mut c, fichas::current_epoch(), &blinded).await.err().as_deref(), Some(FICHAS_OFF));
}

// ------------------------------------------------------------------ resgate

struct CallSide {
    sock: UdpSocket,
    room: RoomKeys,
    role: Role,
}

async fn call_relay(spent_prefix: &str) -> (std::net::SocketAddr, Vec<u8>) {
    let keys = qt_transport::generate_static_keypair().unwrap();
    let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = udp.local_addr().unwrap();
    let dir = std::env::temp_dir().join(format!("qt-fichas-e2e-{}-{}", std::process::id(), addr.port()));
    std::fs::create_dir_all(&dir).unwrap();
    let seed_path = dir.join("fichas_seed.bin");
    std::fs::write(&seed_path, SEED).unwrap();
    let spent = Arc::new(SpentStore::with_prefix("redis://127.0.0.1:6379", spent_prefix).unwrap());
    serve(udp, None, &keys.private, CallLimits::default(), FichasSetup { seed_path: Some(seed_path), spent: Some(spent) });
    (addr, keys.public)
}

async fn recv(s: &UdpSocket) -> Vec<u8> {
    let mut buf = vec![0u8; 2048];
    let (n, _) = tokio::time::timeout(Duration::from_secs(3), s.recv_from(&mut buf)).await.expect("nada chegou").unwrap();
    buf.truncate(n);
    buf
}

async fn join(relay: std::net::SocketAddr, pk: &[u8], secret: u8, role: Role) -> CallSide {
    let room = RoomKeys::derive(&[secret; 32], &[secret; 16]);
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut p = vec![JOIN_KIND];
    p.extend(qt_transport::noise_n_seal(pk, JOIN_PROLOGUE, &join_payload(role, &room)).unwrap());
    sock.send_to(&p, relay).await.unwrap();
    assert!(parse_join_ok(&room, &recv(&sock).await).is_some());
    CallSide { sock, room, role }
}

/// O que sai do telemovel para o relay das chamadas: o pacote do resgate, e o que o relay
/// le depois de o decifrar (devolvido para o teste ver exatamente o que o relay recebe).
fn redeem_packet(pk: &[u8], s: &CallSide, f: &Ficha) -> (Vec<u8>, Vec<u8>) {
    let payload = fichas::redeem_payload(s.role, &s.room, f).to_vec();
    let mut p = vec![REDEEM_KIND];
    p.extend(qt_transport::noise_n_seal(pk, REDEEM_PROLOGUE, &payload).unwrap());
    (p, payload)
}

async fn redeem(relay: std::net::SocketAddr, pk: &[u8], s: &CallSide, f: &Ficha) -> (fichas::RedeemAck, Vec<u8>) {
    let (p, seen) = redeem_packet(pk, s, f);
    s.sock.send_to(&p, relay).await.unwrap();
    (fichas::parse_redeem_ack(&s.room, &recv(&s.sock).await).expect("resposta do relay"), seen)
}

#[tokio::test]
async fn ficha_aceite_uma_vez_e_o_relay_nao_liga_resgate_a_emissao() {
    let relay = TestRelay::start(cfg(5)).await;
    let alice = SigningKeyPair::generate();
    registar(&relay, &uid(&alice)).await;
    let mut a = client(&relay, &alice).await;
    let (f, ans, blinded) = issue(&mut a, 3).await;

    let prefix = format!("fichas-t-{}-{}", std::process::id(), relay.port);
    let (addr, pk) = call_relay(&prefix).await;
    let s1 = join(addr, &pk, 1, Role::Caller).await;
    let s2 = join(addr, &pk, 2, Role::Callee).await;

    // Aceite uma vez (um bloco de 60 s).
    let (k, seen) = redeem(addr, &pk, &s1, &f[0]).await;
    assert_eq!((k.status, k.credit_secs), (RedeemStatus::Accepted, 60));
    // A mesma ficha noutra sala: ja gasta (registo no Redis), sem credito.
    let (k, _) = redeem(addr, &pk, &s2, &f[0]).await;
    assert_eq!((k.status, k.credit_secs), (RedeemStatus::Spent, 0));
    // Outra ficha da mesma conta noutra sala: aceite (o relay nao sabe que e a mesma conta).
    let (k, seen2) = redeem(addr, &pk, &s2, &f[1]).await;
    assert_eq!(k.status, RedeemStatus::Accepted);

    // ---- O que o relay das chamadas recebe nao tem nada da conta nem da emissao. ----
    // O conteudo do resgate e so: versao, papel, sala, relay_key da sala, mes, nonce, saida.
    assert_eq!(seen.len(), fichas::REDEEM_PAYLOAD_LEN);
    let account: Vec<Vec<u8>> = vec![
        alice.verifying_key.to_bytes().to_vec(),
        uid(&alice).into_bytes(),
        blinded.clone(),
        ans.evaluated.clone(),
        ans.proof.clone(),
    ];
    for x in [&seen, &seen2] {
        for secret in &account {
            for w in secret.windows(8) {
                assert!(!x.windows(8).any(|y| y == w), "o resgate contem bytes da conta ou da emissao");
            }
        }
    }
    // O emissor so guardou a contagem desta conta: nem os elementos cegos, nem a resposta.
    let mut conn = redis::Client::open("redis://127.0.0.1:6379").unwrap().get_multiplexed_async_connection().await.unwrap();
    let issuer_keys: Vec<String> = conn.keys(format!("{}:fichas:*", relay.prefix)).await.unwrap();
    assert_eq!(issuer_keys, vec![format!("{}:fichas:emitidas:{}:{}", relay.prefix, uid(&alice), fichas::current_epoch())]);
    let count: String = conn.get(&issuer_keys[0]).await.unwrap();
    assert_eq!(count, "3");
    // O relay das chamadas so guardou resumos dos nonces (nada da conta, da sala ou do IP).
    let mut spent: Vec<String> = conn.keys(format!("{prefix}:gastas:*")).await.unwrap();
    spent.sort();
    let mut expected: Vec<String> = [&f[0], &f[1]]
        .iter()
        .map(|x| format!("{prefix}:gastas:{}:{}", x.epoch, fichas::spent_id(&x.nonce).iter().map(|b| format!("{b:02x}")).collect::<String>()))
        .collect();
    expected.sort();
    assert_eq!(spent, expected);
    let ttl: i64 = conn.ttl(&spent[0]).await.unwrap();
    assert!(ttl > 86_400 && ttl <= 32 * 86_400, "apagadas sozinhas depois do fim do mes: {ttl}");
    for k in &spent {
        assert!(!k.contains(&uid(&alice)));
        let _: () = conn.del(k).await.unwrap();
    }
    let _: () = conn.del(&issuer_keys[0]).await.unwrap();
}

#[tokio::test]
async fn registo_das_gastas_no_redis() {
    let store = SpentStore::with_prefix("redis://127.0.0.1:6379", &format!("fichas-r-{}", std::process::id())).unwrap();
    let id = [7u8; 16];
    let e = fichas::current_epoch();
    assert!(store.mark(e, &id).await.unwrap(), "primeira vez");
    assert!(!store.mark(e, &id).await.unwrap(), "repetida");
    assert!(store.mark(e, &[8u8; 16]).await.unwrap());
    // Por epoca: o mesmo resumo noutro mes e outra entrada (as fichas sao de chaves diferentes).
    assert!(store.mark(190001, &id).await.unwrap());
    // Redis em baixo: erro (o relay trata como "nao verificavel" e nao da video).
    let down = SpentStore::new("redis://127.0.0.1:1").unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(5), down.mark(e, &id)).await.map(|r| r.is_err()).unwrap_or(true));
}
