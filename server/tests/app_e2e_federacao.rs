//! Como os telemoveis usam a federacao: contas em relays diferentes, recuperar uma conta a partir
//! do relay principal, relay fechado a contas novas. Relays, federacao e nucleo verdadeiros.

mod common;
#[path = "common/telemovel.rs"]
mod telemovel;

use common::*;
use qt_server::federation::{FedConfig, Federacao, ParConfig};
use qt_server::{handle_connection_with, ServerConfig, Store};
use std::sync::Arc;
use std::time::Duration;
use telemovel::*;
use tokio::net::TcpListener;

struct Lado {
    nome: String,
    prefix: String,
    chave: qt_transport::NoiseStaticKeyPair,
    porta: u16,
    ouvinte: Option<TcpListener>,
    store: Arc<Store>,
}

impl Lado {
    async fn novo(nome: &str) -> Self {
        let ouvinte = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let prefix = format!("appfed-{}-{}-{nome}", std::process::id(), ouvinte.local_addr().unwrap().port());
        Lado {
            nome: nome.into(),
            store: Arc::new(Store::with_prefix("redis://127.0.0.1:6379", &prefix).unwrap()),
            prefix,
            chave: qt_transport::generate_static_keypair().unwrap(),
            porta: ouvinte.local_addr().unwrap().port(),
            ouvinte: Some(ouvinte),
        }
    }
    fn como_par(&self) -> ParConfig {
        let hex: String = self.chave.public.iter().map(|x| format!("{x:02x}")).collect();
        ParConfig { nome: self.nome.clone(), url: format!("ws://127.0.0.1:{}", self.porta), chave: hex, ip: None }
    }
    fn arrancar(&mut self, pares: Vec<ParConfig>) -> Arc<Federacao> {
        let cfg = FedConfig { nome: self.nome.clone(), pares };
        Federacao::arrancar(cfg, self.store.clone(), self.chave.private.clone(), self.ouvinte.take().unwrap())
    }
    fn redis(&self) -> redis::Connection {
        redis::Client::open("redis://127.0.0.1:6379").unwrap().get_connection().unwrap()
    }
    /// A conta tem marca de idade neste relay (foi criada aqui)? E ha quantos segundos.
    fn idade(&self, uid: &str) -> Option<i64> {
        let ttl: i64 = redis::cmd("TTL").arg(format!("{}:idade:{uid}", self.prefix)).query(&mut self.redis()).unwrap();
        (ttl > 0).then(|| qt_server::store::AGE_MARK_SECONDS - ttl)
    }
    /// Faz de conta que a conta foi criada aqui ha `dias` dias.
    fn envelhecer(&self, uid: &str, dias: i64) {
        let ttl = qt_server::store::AGE_MARK_SECONDS - dias * 86_400;
        let ok: i64 = redis::cmd("EXPIRE").arg(format!("{}:idade:{uid}", self.prefix)).arg(ttl).query(&mut self.redis()).unwrap();
        assert_eq!(ok, 1, "a conta nao tem marca de idade aqui");
    }
    fn tem_conta(&self, uid: &str) -> bool {
        let n: i64 = redis::cmd("EXISTS").arg(format!("{}:bundle:{uid}", self.prefix)).query(&mut self.redis()).unwrap();
        n == 1
    }
}

async fn relay_com(lado: &Lado, cfg: ServerConfig) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let keys = qt_transport::generate_static_keypair().unwrap();
    let s = lado.store.clone();
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

/// Suecia (principal) e Espanha, federados. `fechar_suecia`: Suecia sem contas novas.
async fn federacao(fechar_suecia: bool) -> (Lado, Lado, Vec<(String, String)>) {
    let mut se = Lado::novo("estocolmo").await;
    let mut es = Lado::novo("espanha").await;
    let (pse, pes) = (se.como_par(), es.como_par());
    let fse = se.arrancar(vec![pes]);
    let fes = es.arrancar(vec![pse]);
    for _ in 0..200 {
        if fse.pares().iter().all(|p| p.ligado()) && fes.pares().iter().all(|p| p.ligado()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let use_ = relay_com(&se, ServerConfig { federacao: Some(fse), novas_contas: !fechar_suecia, ..fast_cfg() }).await;
    let ues = relay_com(&es, ServerConfig { federacao: Some(fes), ..fast_cfg() }).await;
    (se, es, vec![("estocolmo".into(), use_), ("espanha".into(), ues)])
}

async fn tel(nome: &str, relays: &[(String, String)], casa: usize) -> Telemovel {
    let mut t = Telemovel::novo(nome, &relays[casa].1);
    t.relays = relays.to_vec();
    t.criar_conta().await;
    t
}

fn cid(t: &Telemovel) -> String {
    t.como_contacto("x").contact_id()
}

fn uid(t: &Telemovel) -> String {
    t.qt.relay_user_id().unwrap()
}

#[tokio::test]
async fn contas_em_relays_diferentes_e_recuperar_a_partir_do_principal() {
    let (se, es, relays) = federacao(false).await;
    let mut a = tel("ana", &relays, 1).await; // conta em Espanha
    let mut b = tel("bruno", &relays, 0).await; // conta na Suecia
    a.adicionar(b.como_contacto("Bruno"));
    b.adicionar(a.como_contacto("Ana"));
    let (ca, cb) = (cid(&a), cid(&b));
    a.enviar(&cb, "de espanha").await.unwrap();
    for _ in 0..3 { ate_acalmar(&mut [&mut a, &mut b]).await; }
    assert_eq!(b.recebidas(&ca), ["de espanha"]);
    b.enviar(&ca, "da suecia").await.unwrap();
    for _ in 0..3 { ate_acalmar(&mut [&mut a, &mut b]).await; }
    assert_eq!(a.recebidas(&cb), ["da suecia"]);

    // As duas contas ja tem dias: so assim se ve se a recuperacao as faz "novas" outra vez.
    es.envelhecer(&uid(&a), 20);
    se.envelhecer(&uid(&b), 20);
    let pa = a.ativar_recuperacao().await;
    let pb = b.ativar_recuperacao().await;
    // Os dois desinstalam; a app nova comeca sempre no relay principal (Suecia).
    a.desinstalar();
    b.desinstalar();
    a.recuperar(pa).await.unwrap();
    b.recuperar(pb).await.unwrap();
    assert_eq!(a.url, relays[1].1, "a conta da Ana continua em Espanha");
    assert_eq!(b.url, relays[0].1);
    assert!(!se.tem_conta(&uid(&a)), "a recuperacao nao pode criar a conta da Ana na Suecia");
    let (ia, ib) = (es.idade(&uid(&a)), se.idade(&uid(&b)));
    println!("idade depois de recuperar: Ana {ia:?} s, Bruno {ib:?} s");
    assert!(ia.unwrap_or(i64::MAX) > 86_400 && ib.unwrap_or(i64::MAX) > 86_400, "recuperar nao pode tornar a conta nova (limites de conta nova)");
    for _ in 0..4 { ate_acalmar(&mut [&mut a, &mut b]).await; }
    a.enviar(&cb, "depois a").await.unwrap();
    b.enviar(&ca, "depois b").await.unwrap();
    for _ in 0..4 { ate_acalmar(&mut [&mut a, &mut b]).await; }
    assert!(b.recebidas(&ca).contains(&"depois a".to_string()), "{:?}", b.recebidas(&ca));
    assert!(a.recebidas(&cb).contains(&"depois b".to_string()), "{:?}", a.recebidas(&cb));
}

#[tokio::test]
async fn recuperar_com_o_principal_fechado_a_contas_novas() {
    let (se, es, relays) = federacao(true).await;
    let mut a = tel("ana", &relays, 1).await;
    let mut b = tel("bruno", &relays, 1).await;
    assert_eq!(a.url, relays[1].1, "conta nova vai para Espanha (Suecia fechada)");
    a.adicionar(b.como_contacto("Bruno"));
    b.adicionar(a.como_contacto("Ana"));
    let (ca, cb) = (cid(&a), cid(&b));
    es.envelhecer(&uid(&a), 20);
    let pa = a.ativar_recuperacao().await;
    a.desinstalar();
    a.recuperar(pa).await.unwrap();
    assert_eq!(a.url, relays[1].1);
    assert!(!se.tem_conta(&uid(&a)));
    println!("idade da Ana: {:?}", es.idade(&uid(&a)));
    assert!(es.idade(&uid(&a)).unwrap_or(i64::MAX) > 86_400);
    for _ in 0..3 { ate_acalmar(&mut [&mut a, &mut b]).await; }
    a.enviar(&cb, "ok").await.unwrap();
    b.enviar(&ca, "ok2").await.unwrap();
    for _ in 0..3 { ate_acalmar(&mut [&mut a, &mut b]).await; }
    assert!(b.recebidas(&ca).contains(&"ok".to_string()));
    assert!(a.recebidas(&cb).contains(&"ok2".to_string()));
}
