//! Copias de recuperacao nos dois relays (`replicas.rs`): recuperar com o relay da conta parado,
//! apagar e mudar com um relay em baixo, e voltar a casa sem perder mensagens. Os dois relays
//! ligam-se por uma "ponte" que se corta para fingir que um esta em baixo.
//! Precisa de um Redis em 127.0.0.1:6379.

mod common;
use common::*;

use base64::Engine;
use qt_core::primitives::SigningKeyPair;
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::federation::{Estado, FedConfig, Federacao, ParConfig};
use qt_server::protocol::*;
use qt_server::{handle_connection_with, ServerConfig, Store};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

static SEQ: AtomicU32 = AtomicU32::new(0);

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn b64(d: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(d)
}

/// Passa ligacoes TCP para `destino`; cortada, fecha as que ha e recusa as novas.
async fn ponte(destino: u16) -> (u16, watch::Sender<bool>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let porta = listener.local_addr().unwrap().port();
    let (tx, rx) = watch::channel(true);
    tokio::spawn(async move {
        loop {
            let Ok((mut entra, _)) = listener.accept().await else { break };
            let mut rx = rx.clone();
            if !*rx.borrow() {
                continue;
            }
            tokio::spawn(async move {
                let Ok(mut sai) = TcpStream::connect(("127.0.0.1", destino)).await else { return };
                tokio::select! {
                    _ = tokio::io::copy_bidirectional(&mut entra, &mut sai) => {}
                    _ = async { while *rx.borrow_and_update() { if rx.changed().await.is_err() { break } } } => {}
                }
            });
        }
    });
    (porta, tx)
}

struct Relay {
    prefix: String,
    store: Arc<Store>,
    fed: Arc<Federacao>,
    url: String,
}

struct Dois {
    a: Relay,
    b: Relay,
    pontes: [watch::Sender<bool>; 2],
}

impl Dois {
    async fn novos() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let (oa, ob) = (TcpListener::bind("127.0.0.1:0").await.unwrap(), TcpListener::bind("127.0.0.1:0").await.unwrap());
        // a -> b passa pela ponte_b; b -> a pela ponte_a.
        let (pb, tb) = ponte(ob.local_addr().unwrap().port()).await;
        let (pa, ta) = ponte(oa.local_addr().unwrap().port()).await;
        let (ka, kb) = (qt_transport::generate_static_keypair().unwrap(), qt_transport::generate_static_keypair().unwrap());
        let par = |nome: &str, porta: u16, chave: &[u8]| ParConfig {
            nome: nome.into(),
            url: format!("ws://127.0.0.1:{porta}"),
            chave: hex(chave),
            ip: None,
        };
        let (cfg_a, cfg_b) = (
            FedConfig { nome: "a".into(), pares: vec![par("b", pb, &kb.public)] },
            FedConfig { nome: "b".into(), pares: vec![par("a", pa, &ka.public)] },
        );
        let a = relay("a", n, cfg_a, ka.private.clone(), oa).await;
        let b = relay("b", n, cfg_b, kb.private.clone(), ob).await;
        let d = Dois { a, b, pontes: [ta, tb] };
        d.ligados().await;
        d
    }

    async fn ligados(&self) {
        for _ in 0..400 {
            if self.a.fed.pares()[0].ligado() && self.b.fed.pares()[0].ligado() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("os relays nao se ligaram");
    }

    /// Um relay "em baixo" para o outro.
    async fn cortar(&self) {
        for p in &self.pontes {
            p.send_replace(false);
        }
        for _ in 0..200 {
            if !self.a.fed.pares()[0].ligado() && !self.b.fed.pares()[0].ligado() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("a ponte nao fechou");
    }

    async fn repor(&self) {
        for p in &self.pontes {
            p.send_replace(true);
        }
        self.ligados().await;
    }
}

async fn relay(nome: &str, n: u32, cfg: FedConfig, chave: Vec<u8>, ouvinte: TcpListener) -> Relay {
    let prefix = format!("copias-{}-{n}-{nome}", std::process::id());
    let mut st = Store::with_prefix("redis://127.0.0.1:6379", &prefix).unwrap();
    enable_test_backups(&mut st, &prefix);
    let store = Arc::new(st);
    cfg.validar().unwrap();
    let fed = Federacao::arrancar(cfg, store.clone(), chave, ouvinte);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let keys = qt_transport::generate_static_keypair().unwrap();
    let (s, c) = (store.clone(), ServerConfig { federacao: Some(fed.clone()), ..fast_cfg() });
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { break };
            let (s, k, c) = (s.clone(), keys.private.clone(), c.clone());
            tokio::spawn(async move {
                let _ = handle_connection_with(tcp, s, k, &c).await;
            });
        }
    });
    Relay { prefix, store, fed, url }
}

async fn ligar(url: &str, s: &SigningKeyPair) -> RelayClient {
    let cfg = RelayConfig {
        connect_timeout: Duration::from_secs(3),
        request_timeout: Duration::from_secs(10),
        max_retries: 0,
        backoff_base: Duration::from_millis(50),
        expected_server_static: None,
    };
    let n = qt_transport::generate_static_keypair().unwrap();
    RelayClient::connect(url, n.private, s.to_bytes(), cfg).await.unwrap()
}

fn uid(s: &SigningKeyPair) -> String {
    user_id_from_signing_public(&s.verifying_key.to_bytes())
}

async fn registar_em(c: &mut RelayClient, s: &SigningKeyPair) -> ServerMessage {
    let pow = qt_core::pow::pow_solve(&s.verifying_key.to_bytes(), qt_core::pow::POW_BITS);
    c.request(ClientMessage::RegisterPreKeyBundle { user_id: uid(s), bundle_bytes: valid_bundle(s, 1), pow, one_time_pre_keys: None })
        .await
        .unwrap()
}

/// Conta ja existente neste relay. (Registar uma nova deixava o outro relay com uma reserva de
/// 60 s, que numa conta antiga ja passou.)
async fn conta(r: &Relay) -> (SigningKeyPair, RelayClient) {
    let s = SigningKeyPair::generate();
    r.store.register_bundle(uid(&s), valid_bundle(&s, 1)).await.unwrap();
    let c = ligar(&r.url, &s).await;
    (s, c)
}

fn ack(r: ServerMessage) {
    assert!(matches!(r, ServerMessage::Ack), "esperava Ack, veio {r:?}");
}

fn recusa(r: ServerMessage, motivo: &str) {
    match r {
        ServerMessage::Error { reason } => assert_eq!(reason, motivo),
        other => panic!("esperava Error({motivo}), veio {other:?}"),
    }
}

/// Copia de duas partes (uma cheia), como a app a envia.
async fn gravar(c: &mut RelayClient, lookup: &str, generation: u8) {
    for (part, data) in [vec![generation; BACKUP_PART_MAX], vec![generation; 100]].iter().enumerate() {
        ack(c
            .request(ClientMessage::BackupPut {
                lookup: lookup.into(),
                generation: format!("{generation:02x}").repeat(8),
                part: part as u32,
                parts: 2,
                data: b64(data),
            })
            .await
            .unwrap());
    }
}

/// Geracao e tamanhos da copia neste relay.
async fn copia_em(r: &Relay, lookup: &str) -> Option<(u8, usize)> {
    // Pode estar a ser trocada ou apagada neste instante: nesse caso volta a perguntar-se.
    let (parts, first) = r.store.backup_get(lookup, 0).await.ok()??;
    let mut total = first.len();
    for i in 1..parts {
        total += r.store.backup_get(lookup, i).await.ok()??.1.len();
    }
    Some((first[0], total))
}

async fn esperar_copia(r: &Relay, lookup: &str, esperado: Option<(u8, usize)>) {
    for _ in 0..200 {
        if copia_em(r, lookup).await == esperado {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{}: a copia ficou {:?} e nao {esperado:?}", r.prefix, copia_em(r, lookup).await);
}

const CHEIA: usize = BACKUP_PART_MAX + 100;

async fn pedir(url: &str, lookup: &str) -> ServerMessage {
    let mut novo = ligar(url, &SigningKeyPair::generate()).await;
    novo.request(ClientMessage::BackupGet { lookup: lookup.into(), part: 0 }).await.unwrap()
}

#[tokio::test]
async fn recupera_com_o_relay_da_conta_parado() {
    let d = Dois::novos().await;
    let (alice, mut c) = conta(&d.a).await;
    let lookup = "a1".repeat(32);
    gravar(&mut c, &lookup, 1).await;
    esperar_copia(&d.b, &lookup, Some((1, CHEIA))).await;

    d.cortar().await;
    // Telemovel novo no relay b: a copia esta la.
    assert!(matches!(pedir(&d.b.url, &lookup).await, ServerMessage::BackupPart { parts: 2, .. }));
    // Palavras sem copia aqui: o b nao sabe o que o a tem, por isso erro e nunca "nao ha".
    let r = pedir(&d.b.url, &"a2".repeat(32)).await;
    assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
    // A conta regista-se no b, mesmo com ele sem aceitar contas novas por estar a por verificar.
    let mut novo = ligar(&d.b.url, &alice).await;
    ack(registar_em(&mut novo, &alice).await);
    assert_eq!(d.b.store.fed_estado(&uid(&alice)).await.unwrap(), Estado::TenhoPorVerificar);

    d.repor().await;
    assert!(matches!(pedir(&d.b.url, &"a2".repeat(32)).await, ServerMessage::BackupNotFound), "com os dois de pe, nao ha");
}

#[tokio::test]
async fn mensagens_nao_se_perdem_quando_o_relay_volta() {
    let d = Dois::novos().await;
    let (alice, mut c) = conta(&d.a).await;
    let (_bob, mut bob) = conta(&d.b).await;
    let lookup = "b1".repeat(32);
    gravar(&mut c, &lookup, 1).await;
    esperar_copia(&d.b, &lookup, Some((1, CHEIA))).await;
    drop(c);

    d.cortar().await;
    let mut no_b = ligar(&d.b.url, &alice).await;
    ack(registar_em(&mut no_b, &alice).await);
    let id = uid(&alice);
    for n in 1..=3u8 {
        ack(bob
            .request(ClientMessage::SendMessage { to: id.clone(), sealed_from: vec![n], ciphertext: vec![n; 64], silent: false, wake: None })
            .await
            .unwrap());
    }

    d.repor().await;
    assert_eq!(d.b.fed.verificar_pendentes().await, (0, 1));
    assert_eq!(d.b.store.fed_estado(&id).await.unwrap(), Estado::NaoTenho);
    assert_eq!(d.b.store.fed_movida(&id).await.unwrap().as_deref(), Some("a"));
    assert_eq!(d.b.fed.encaminhar_pendentes().await, 3);
    assert!(d.b.store.peek_messages(&id, 60_000).await.unwrap().is_empty());
    let la = d.a.store.peek_messages(&id, 60_000).await.unwrap();
    assert_eq!(la.iter().map(|m| m.ciphertext[0]).collect::<Vec<_>>(), vec![1, 2, 3], "no relay da conta, pela ordem");
    // A volta seguinte encontra a fila vazia e tira a conta da lista.
    assert_eq!(d.b.fed.encaminhar_pendentes().await, 0);
    assert!(d.b.store.fed_por_encaminhar(10).await.unwrap().is_empty());

    // A app, ainda ligada ao b, fica a saber para onde mudar (a esperar e a enviar).
    let mut no_b = ligar(&d.b.url, &alice).await;
    recusa(no_b.request(ClientMessage::WaitMessages { user_id: id.clone(), timeout_secs: 20 }).await.unwrap(), &elsewhere(Some("a")));
    recusa(
        no_b.request(ClientMessage::SendMessage { to: uid(&_bob), sealed_from: vec![9], ciphertext: vec![9], silent: false, wake: None })
            .await
            .unwrap(),
        &elsewhere(Some("a")),
    );
    // Registar no b diz o mesmo; no a regista e recebe as mensagens.
    recusa(registar_em(&mut no_b, &alice).await, &elsewhere(Some("a")));
    let mut em_casa = ligar(&d.a.url, &alice).await;
    ack(registar_em(&mut em_casa, &alice).await);
    match em_casa.request(ClientMessage::FetchMessages { user_id: id }).await.unwrap() {
        ServerMessage::Messages { messages } => assert_eq!(messages.len(), 3),
        o => panic!("{o:?}"),
    }
}

#[tokio::test]
async fn mudar_e_apagar_com_um_relay_parado() {
    let d = Dois::novos().await;
    let (alice, mut c) = conta(&d.a).await;
    let (velha, nova) = ("c1".repeat(32), "c2".repeat(32));
    gravar(&mut c, &velha, 1).await;
    esperar_copia(&d.b, &velha, Some((1, CHEIA))).await;

    // Palavras novas com o b em baixo: quando volta, fica a nova e a velha sai.
    d.cortar().await;
    gravar(&mut c, &nova, 2).await;
    assert_eq!(d.a.store.bk_sync_len("b").await.unwrap(), 1, "fica na fila");
    d.repor().await;
    d.a.fed.copia_mudou(&uid(&alice)).await;
    esperar_copia(&d.b, &nova, Some((2, CHEIA))).await;
    esperar_copia(&d.b, &velha, None).await;

    // Desligar a chave de recuperacao com o b em baixo.
    d.cortar().await;
    ack(c.request(ClientMessage::BackupDelete).await.unwrap());
    assert!(copia_em(&d.b, &nova).await.is_some());
    d.repor().await;
    d.a.fed.copia_mudou(&uid(&alice)).await;
    esperar_copia(&d.b, &nova, None).await;
}

#[tokio::test]
async fn apagar_a_conta_apaga_nos_dois() {
    let d = Dois::novos().await;
    let (alice, mut c) = conta(&d.a).await;
    let lookup = "d1".repeat(32);
    gravar(&mut c, &lookup, 1).await;
    esperar_copia(&d.b, &lookup, Some((1, CHEIA))).await;
    ack(c.request(ClientMessage::DeleteAccount { user_id: uid(&alice) }).await.unwrap());
    esperar_copia(&d.b, &lookup, None).await;
}

#[tokio::test]
async fn apagar_a_conta_no_outro_relay_com_o_dela_parado() {
    let d = Dois::novos().await;
    let (alice, mut c) = conta(&d.a).await;
    let id = uid(&alice);
    let lookup = "d2".repeat(32);
    gravar(&mut c, &lookup, 1).await;
    esperar_copia(&d.b, &lookup, Some((1, CHEIA))).await;
    drop(c);
    d.cortar().await;
    let mut no_b = ligar(&d.b.url, &alice).await;
    ack(registar_em(&mut no_b, &alice).await);
    ack(no_b.request(ClientMessage::DeleteAccount { user_id: id.clone() }).await.unwrap());
    d.repor().await;
    d.b.fed.copia_mudou(&id).await;
    esperar_copia(&d.a, &lookup, None).await;
    for _ in 0..100 {
        if d.a.store.fed_estado(&id).await.unwrap() == Estado::Apagada {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("a conta continua no relay dela");
}

#[tokio::test]
async fn copia_antiga_vai_na_revisao_e_o_prazo_acompanha() {
    let d = Dois::novos().await;
    let (alice, mut c) = conta(&d.a).await;
    let id = uid(&alice);
    let lookup = "e1".repeat(32);
    // Como uma copia gravada antes desta versao: so no relay a e fora da fila.
    d.cortar().await;
    gravar(&mut c, &lookup, 1).await;
    while d.a.store.bk_sync_take("b").await.unwrap().is_some() {}
    d.repor().await;
    assert!(copia_em(&d.b, &lookup).await.is_none());
    assert_eq!(d.a.fed.rever_copias().await, 1);
    esperar_copia(&d.b, &lookup, Some((1, CHEIA))).await;
    // Banida: nos dois relays a copia passa a durar 30 dias.
    d.a.store.ban(&id).await.unwrap();
    d.a.store.expire_backup_of_banned(&id).await.unwrap();
    d.a.fed.copia_mudou(&id).await;
    for _ in 0..100 {
        let ttl = d.b.store.backup_state(&id).await.unwrap().copia.map_or(0, |c| c.ttl);
        if ttl > 0 && ttl <= qt_server::store::BANNED_BACKUP_TTL_SECONDS {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("o prazo nao acompanhou");
}

#[tokio::test]
async fn copia_velha_nunca_passa_por_cima_nem_volta() {
    let prefix = format!("copias-{}-ordem", std::process::id());
    let mut s = Store::with_prefix("redis://127.0.0.1:6379", &prefix).unwrap();
    enable_test_backups(&mut s, &prefix);
    let (dono, lookup) = ("11".repeat(32), "f1".repeat(32));
    let ttl = qt_server::store::BACKUP_TTL_SECONDS;
    s.replica_put(&dono, &lookup, &"02".repeat(8), 0, 1, vec![2; 10], 2_000, ttl).await.unwrap();
    // Mais velha: ignorada.
    s.replica_put(&dono, &lookup, &"01".repeat(8), 0, 1, vec![1; 10], 1_000, ttl).await.unwrap();
    assert_eq!(s.backup_get(&lookup, 0).await.unwrap().unwrap().1, vec![2; 10]);
    assert!(!s.replica_check(&dono, &lookup, &"01".repeat(8), 1_000, ttl).await.unwrap());
    s.replica_delete(&dono, 1_500).await.unwrap();
    assert!(s.backup_get(&lookup, 0).await.unwrap().is_some(), "um apagar mais velho nao apaga");
    // Apagada mais tarde: a copia de antes nao volta.
    s.replica_delete(&dono, 3_000).await.unwrap();
    assert!(!s.replica_check(&dono, &lookup, &"02".repeat(8), 2_000, ttl).await.unwrap());
    s.replica_put(&dono, &lookup, &"02".repeat(8), 0, 1, vec![2; 10], 2_000, ttl).await.unwrap();
    assert!(s.backup_get(&lookup, 0).await.unwrap().is_none());
    // Uma nova entra.
    assert!(s.replica_check(&dono, &lookup, &"03".repeat(8), 4_000, ttl).await.unwrap());
}

#[tokio::test]
async fn banida_noutro_relay_enquanto_estava_em_baixo() {
    let d = Dois::novos().await;
    let (alice, c) = conta(&d.a).await;
    let id = uid(&alice);
    drop(c);
    d.cortar().await;
    d.a.store.ban(&id).await.unwrap();
    let mut no_b = ligar(&d.b.url, &alice).await;
    ack(registar_em(&mut no_b, &alice).await);
    d.repor().await;
    d.b.fed.verificar_pendentes().await;
    assert_eq!(d.b.store.fed_estado(&id).await.unwrap(), Estado::Banida);
}
