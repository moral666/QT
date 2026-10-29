//! Federacao fase 1 (`federation.rs`): dois relays em localhost, cada um com o seu Redis
//! (prefixo proprio) e a sua ligacao `/fed`. Descoberta, registo unico, banidas, par em
//! baixo (opcao A1) e chaves erradas. Precisa de um Redis em 127.0.0.1:6379.

mod common;
use common::*;

use qt_core::primitives::SigningKeyPair;
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::federation::{Estado, FedConfig, Federacao, Onde, ParConfig, Reserva};
use qt_server::protocol::*;
use qt_server::{handle_connection_with, ServerConfig, Store};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

static SEQ: AtomicU32 = AtomicU32::new(0);

fn prefixo(nome: &str) -> String {
    format!("fed-{}-{}-{nome}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed))
}

fn store(prefix: &str) -> Arc<Store> {
    Arc::new(Store::with_prefix("redis://127.0.0.1:6379", prefix).unwrap())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Um lado da federacao: chave, porta do `/fed` e Redis.
struct Lado {
    nome: String,
    prefix: String,
    chave: qt_transport::NoiseStaticKeyPair,
    porta: u16,
    ouvinte: Option<TcpListener>,
}

impl Lado {
    async fn novo(nome: &str) -> Self {
        let ouvinte = TcpListener::bind("127.0.0.1:0").await.unwrap();
        Lado {
            nome: nome.into(),
            prefix: prefixo(nome),
            chave: qt_transport::generate_static_keypair().unwrap(),
            porta: ouvinte.local_addr().unwrap().port(),
            ouvinte: Some(ouvinte),
        }
    }

    fn como_par(&self) -> ParConfig {
        ParConfig {
            nome: self.nome.clone(),
            url: format!("ws://127.0.0.1:{}", self.porta),
            chave: hex(&self.chave.public),
            ip: None,
        }
    }

    fn arrancar(&mut self, pares: Vec<ParConfig>) -> Arc<Federacao> {
        let cfg = FedConfig { nome: self.nome.clone(), pares };
        cfg.validar().unwrap();
        Federacao::arrancar(cfg, store(&self.prefix), self.chave.private.clone(), self.ouvinte.take().unwrap())
    }
}

/// Relay das apps com esta federacao, no Redis `prefix`. Devolve o URL.
async fn relay(prefix: &str, fed: Option<Arc<Federacao>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let keys = qt_transport::generate_static_keypair().unwrap();
    let cfg = ServerConfig { federacao: fed, ..fast_cfg() };
    let s = store(prefix);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { break };
            let (s, k, c) = (s.clone(), keys.private.clone(), cfg.clone());
            tokio::spawn(async move {
                let _ = handle_connection_with(tcp, s, k, &c).await;
            });
        }
    });
    format!("ws://127.0.0.1:{port}")
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

/// Registo como as apps (com prova de trabalho). Devolve a resposta do relay.
async fn registar_em(url: &str, s: &SigningKeyPair) -> ServerMessage {
    let mut c = ligar(url, s).await;
    let pow = qt_core::pow::pow_solve(&s.verifying_key.to_bytes(), qt_core::pow::POW_BITS);
    c.request(ClientMessage::RegisterPreKeyBundle { user_id: uid(s), bundle_bytes: valid_bundle(s, 1), pow, one_time_pre_keys: None })
        .await
        .unwrap()
}

fn recusa(r: ServerMessage, motivo: &str) {
    match r {
        ServerMessage::Error { reason } => assert_eq!(reason, motivo),
        other => panic!("esperava Error({motivo}), veio {other:?}"),
    }
}

fn ack(r: ServerMessage) {
    assert!(matches!(r, ServerMessage::Ack), "esperava Ack, veio {r:?}");
}

async fn esperar_ligado(fed: &Federacao) {
    for _ in 0..200 {
        if fed.pares().iter().all(|p| p.ligado()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("os pares nao se ligaram");
}

/// Dois relays federados (a, b), ja ligados um ao outro.
async fn par_federado() -> (Lado, Arc<Federacao>, String, Lado, Arc<Federacao>, String) {
    let mut a = Lado::novo("a").await;
    let mut b = Lado::novo("b").await;
    let (pa, pb) = (a.como_par(), b.como_par());
    let fa = a.arrancar(vec![pb]);
    let fb = b.arrancar(vec![pa]);
    esperar_ligado(&fa).await;
    esperar_ligado(&fb).await;
    let (ua, ub) = (relay(&a.prefix, Some(fa.clone())).await, relay(&b.prefix, Some(fb.clone())).await);
    (a, fa, ua, b, fb, ub)
}

#[tokio::test]
async fn uma_conta_so_existe_num_relay() {
    let (a, fa, ua, b, _fb, ub) = par_federado().await;
    let alice = SigningKeyPair::generate();
    ack(registar_em(&ua, &alice).await);
    // A mesma conta (ex.: recuperada com as 12 palavras) noutro relay: recusada.
    recusa(registar_em(&ub, &alice).await, ELSEWHERE);
    assert_eq!(store(&b.prefix).fed_estado(&uid(&alice)).await.unwrap(), Estado::NaoTenho);
    // No relay de casa, renovar continua a funcionar e nao fica nada por verificar.
    ack(registar_em(&ua, &alice).await);
    assert_eq!(store(&a.prefix).fed_estado(&uid(&alice)).await.unwrap(), Estado::Tenho);
    assert!(store(&a.prefix).fed_por_verificar(10).await.unwrap().is_empty());
    // Contas diferentes registam-se nos dois relays sem problema.
    ack(registar_em(&ub, &SigningKeyPair::generate()).await);
    drop(fa);
}

#[tokio::test]
async fn descoberta_com_cache() {
    let (a, _fa, ua, _b, fb, _ub) = par_federado().await;
    let alice = SigningKeyPair::generate();
    ack(registar_em(&ua, &alice).await);
    let id = uid(&alice);
    assert_eq!(fb.onde_esta(&id).await, Onde::Par("a".into()));
    let inventado = uid(&SigningKeyPair::generate());
    assert_eq!(fb.onde_esta(&inventado).await, Onde::Nenhum);
    // Varias de uma vez, pela mesma ordem.
    assert_eq!(fb.onde_estao(&[inventado.clone(), id.clone()]).await, vec![Onde::Nenhum, Onde::Par("a".into())]);
    // A Alice apaga a conta: o b continua com a resposta guardada ate a esquecer.
    store(&a.prefix).delete_account(&id).await.unwrap();
    assert_eq!(fb.onde_esta(&id).await, Onde::Par("a".into()), "cache positiva");
    fb.esquecer(&id);
    assert_eq!(fb.onde_esta(&id).await, Onde::Apagada);
    // IDs invalidos nao chegam aos pares.
    assert_eq!(fb.onde_esta("xyz").await, Onde::Nenhum);
}

#[tokio::test]
async fn banida_num_relay_nao_regista_noutro() {
    let (a, _fa, _ua, _b, fb, ub) = par_federado().await;
    let m = SigningKeyPair::generate();
    store(&a.prefix).ban(&uid(&m)).await.unwrap();
    recusa(registar_em(&ub, &m).await, BANNED);
    assert_eq!(fb.onde_esta(&uid(&m)).await, Onde::Banida);
}

#[tokio::test]
async fn reserva_ao_mesmo_tempo_nos_dois_relays() {
    let (_a, fa, _ua, _b, fb, _ub) = par_federado().await;
    let id = uid(&SigningKeyPair::generate());
    let (ra, rb) = tokio::join!(fa.reservar(&id), fb.reservar(&id));
    let livres = [ra, rb].iter().filter(|r| matches!(r, Reserva::Livre { .. })).count();
    assert!(livres <= 1, "os dois relays nao podem ficar com a conta: {ra:?} {rb:?}");
    fa.libertar(&id);
    fb.libertar(&id);
}

/// Opcao A1: com o par em baixo, regista e verifica quando ele voltar.
#[tokio::test]
async fn par_em_baixo_regista_e_verifica_depois() {
    let mut a = Lado::novo("a").await;
    let mut b = Lado::novo("b").await;
    let (pa, pb) = (a.como_par(), b.como_par());
    // O `a` ainda nao ouve: a porta fica fechada ate ele arrancar.
    let ouvinte_a = a.ouvinte.take().unwrap();
    drop(ouvinte_a);
    let fb = b.arrancar(vec![pa]);
    let ub = relay(&b.prefix, Some(fb.clone())).await;
    let sb = store(&b.prefix);

    // Duas contas novas no b com o a em baixo: as duas ficam por verificar.
    let (x, y) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    ack(registar_em(&ub, &x).await);
    ack(registar_em(&ub, &y).await);
    assert_eq!(sb.fed_estado(&uid(&x)).await.unwrap(), Estado::TenhoPorVerificar);
    // Renovar nao pergunta nada nem muda a marca.
    ack(registar_em(&ub, &x).await);
    assert_eq!(sb.fed_por_verificar(10).await.unwrap().len(), 2);
    // Com o par em baixo nada se decide.
    assert_eq!(fb.verificar_pendentes().await, (0, 0));
    assert_eq!(fb.onde_esta(&uid(&SigningKeyPair::generate())).await, Onde::Incerto);

    // O `a` ja tinha a conta x (ex.: telemovel antigo). Volta a ligar-se.
    store(&a.prefix).register_bundle(uid(&x), vec![0xB0]).await.unwrap();
    a.ouvinte = Some(TcpListener::bind(("127.0.0.1", a.porta)).await.unwrap());
    let fa = a.arrancar(vec![pb]);
    esperar_ligado(&fb).await;
    esperar_ligado(&fa).await;
    // x estava no a: o b larga-a. y so existe no b: fica verificada.
    assert_eq!(fb.verificar_pendentes().await, (1, 1));
    assert_eq!(sb.fed_estado(&uid(&x)).await.unwrap(), Estado::NaoTenho);
    assert_eq!(sb.fed_estado(&uid(&y)).await.unwrap(), Estado::Tenho);
    assert!(sb.fed_por_verificar(10).await.unwrap().is_empty());
    recusa(registar_em(&ub, &x).await, ELSEWHERE);
}

/// Os dois registaram a mesma conta sem verificar: fica no relay de nome menor ("a").
#[tokio::test]
async fn os_dois_por_verificar_fica_no_nome_menor() {
    let (a, fa, _ua, b, fb, _ub) = par_federado().await;
    let id = uid(&SigningKeyPair::generate());
    for p in [&a.prefix, &b.prefix] {
        let s = store(p);
        s.register_bundle(id.clone(), vec![0xB0]).await.unwrap();
        s.fed_marcar_por_verificar(&id).await.unwrap();
    }
    assert_eq!(fb.verificar_pendentes().await, (0, 1), "o b larga");
    assert_eq!(fa.verificar_pendentes().await, (1, 0), "o a fica");
    assert_eq!(store(&a.prefix).fed_estado(&id).await.unwrap(), Estado::Tenho);
    assert_eq!(store(&b.prefix).fed_estado(&id).await.unwrap(), Estado::NaoTenho);
}

#[tokio::test]
async fn chave_fora_da_lista_ou_pin_errado_nao_liga() {
    let mut a = Lado::novo("a").await;
    let mut b = Lado::novo("b").await;
    let mut c = Lado::novo("c").await;
    // O `a` so conhece o `b`. O `c` diz-se "b" ao `a`, mas a chave dele nao esta na lista.
    let fa = a.arrancar(vec![b.como_par()]);
    let fc = Federacao::arrancar(
        FedConfig { nome: "b".into(), pares: vec![a.como_par()] },
        store(&c.prefix),
        c.chave.private.clone(),
        c.ouvinte.take().unwrap(),
    );
    // O `b` verdadeiro conhece o `a` com o pin errado.
    let mut a_pin_errado = a.como_par();
    a_pin_errado.chave = hex(&qt_transport::generate_static_keypair().unwrap().public);
    let fb = b.arrancar(vec![a_pin_errado]);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!fc.pares()[0].ligado(), "chave fora da lista do a");
    assert!(!fb.pares()[0].ligado(), "pin do a errado");
    // O `b` tem o `a` com outra chave: tambem recusa a ligacao que vem do `a`.
    assert!(!fa.pares()[0].ligado(), "o b recusa a chave verdadeira do a (nao e a da lista dele)");
    assert_eq!(fc.onde_esta(&uid(&SigningKeyPair::generate())).await, Onde::Incerto);
}
