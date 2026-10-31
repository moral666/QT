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

/// Um lado da federacao: chave, porta do `/fed` e Redis (com anexos, numa pasta propria).
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
        let prefix = prefixo(nome);
        let mut st = Store::with_prefix("redis://127.0.0.1:6379", &prefix).unwrap();
        let mut b = qt_server::BlobConfig::new(std::env::temp_dir().join(format!("qt-fed-anexos-{prefix}")));
        b.min_free_bytes = 0;
        st.enable_blobs(b).unwrap();
        Lado {
            nome: nome.into(),
            prefix,
            chave: qt_transport::generate_static_keypair().unwrap(),
            porta: ouvinte.local_addr().unwrap().port(),
            ouvinte: Some(ouvinte),
            store: Arc::new(st),
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
        Federacao::arrancar(cfg, self.store.clone(), self.chave.private.clone(), self.ouvinte.take().unwrap())
    }
}

/// Relay das apps deste lado, com esta federacao. Devolve o URL.
async fn relay(lado: &Lado, fed: Option<Arc<Federacao>>) -> String {
    relay_com(lado, ServerConfig { federacao: fed, ..fast_cfg() }).await
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
    let (ua, ub) = (relay(&a, Some(fa.clone())).await, relay(&b, Some(fb.clone())).await);
    (a, fa, ua, b, fb, ub)
}

#[tokio::test]
async fn uma_conta_so_existe_num_relay() {
    let (a, fa, ua, b, _fb, ub) = par_federado().await;
    let alice = SigningKeyPair::generate();
    ack(registar_em(&ua, &alice).await);
    // A mesma conta (ex.: recuperada com as 12 palavras) noutro relay: recusada.
    recusa(registar_em(&ub, &alice).await, &elsewhere(Some("a")));
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
    let livres = [ra.clone(), rb.clone()].iter().filter(|r| matches!(r, Reserva::Livre { .. })).count();
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
    let ub = relay(&b, Some(fb.clone())).await;
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
    recusa(registar_em(&ub, &x).await, &elsewhere(Some("a")));
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

// ---------------------------------------------------------------------------
// Fase 2: mensagens, bundles, anexos e copias entre relays
// ---------------------------------------------------------------------------

/// Conta registada (com bundle e one-time prekeys) e ligada ao relay `url`.
async fn conta(url: &str) -> (SigningKeyPair, RelayClient) {
    let s = SigningKeyPair::generate();
    let mut c = ligar(url, &s).await;
    let pow = qt_core::pow::pow_solve(&s.verifying_key.to_bytes(), qt_core::pow::POW_BITS);
    let otpks = vec![[[0, 0, 0, 1].as_slice(), &[7u8; 32]].concat(), [[0, 0, 0, 2].as_slice(), &[8u8; 32]].concat()];
    ack(c
        .request(ClientMessage::RegisterPreKeyBundle {
            user_id: uid(&s),
            bundle_bytes: valid_bundle(&s, 1),
            pow,
            one_time_pre_keys: Some(otpks),
        })
        .await
        .unwrap());
    (s, c)
}

fn envio(to: &str, n: u8) -> ClientMessage {
    ClientMessage::SendMessage { to: to.into(), sealed_from: vec![n; 80], ciphertext: vec![n; 2000], silent: false, wake: None }
}

#[tokio::test]
async fn mensagens_entre_relays() {
    let (_a, _fa, ua, _b, _fb, ub) = par_federado().await;
    let (_, mut alice) = conta(&ua).await;
    let (bs, mut bob) = conta(&ub).await;
    let bob_id = uid(&bs);
    // A Alice (relay a) escreve ao Bob (relay b); o Bob recebe no relay dele, igual.
    ack(alice.request(envio(&bob_id, 3)).await.unwrap());
    ack(alice.request(envio(&bob_id, 4)).await.unwrap());
    let m = bob.fetch_messages().await.unwrap();
    assert_eq!(m.len(), 2);
    assert_eq!((m[0].sealed_from.clone(), m[0].ciphertext.len()), (vec![3; 80], 2000));
    assert_eq!(m[1].ciphertext[0], 4);
    // E ao contrario, com o Bob ja a espera (entrega imediata no relay b).
    let alice_id = alice.user_id().to_string();
    ack(bob.request(envio(&alice_id, 5)).await.unwrap());
    let r = alice.wait_messages(5).await.unwrap();
    assert_eq!(r[0].ciphertext[0], 5);
    // Um ID que nao existe em lado nenhum continua a dar "destinatario desconhecido".
    recusa(alice.request(envio(&uid(&SigningKeyPair::generate()), 1)).await.unwrap(), RECIPIENT_UNKNOWN);
}

#[tokio::test]
async fn bundle_de_conta_de_outro_relay() {
    let (_a, _fa, ua, b, _fb, ub) = par_federado().await;
    let (_, mut alice) = conta(&ua).await;
    let (bs, _bob) = conta(&ub).await;
    let bob_id = uid(&bs);
    match alice.request(ClientMessage::FetchPreKeyBundle { user_id: bob_id.clone() }).await.unwrap() {
        ServerMessage::PreKeyBundle { bundle_bytes, one_time_pre_key } => {
            assert_eq!(Some(bundle_bytes), b.store.get_bundle(&bob_id).await.unwrap());
            assert_eq!(one_time_pre_key.unwrap()[..4], [0, 0, 0, 1], "a primeira one-time prekey");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(b.store.count_one_time_prekeys(&bob_id).await.unwrap(), 1, "gasta no relay do Bob");
    let inventado = uid(&SigningKeyPair::generate());
    assert!(matches!(
        alice.request(ClientMessage::FetchPreKeyBundle { user_id: inventado }).await.unwrap(),
        ServerMessage::PreKeyBundleNotFound
    ));
}

#[tokio::test]
async fn conta_apagada_noutro_relay() {
    let (_a, _fa, ua, b, _fb, ub) = par_federado().await;
    let (_, mut alice) = conta(&ua).await;
    let (bs, _bob) = conta(&ub).await;
    let bob_id = uid(&bs);
    ack(alice.request(envio(&bob_id, 1)).await.unwrap());
    b.store.delete_account(&bob_id).await.unwrap();
    recusa(alice.request(envio(&bob_id, 2)).await.unwrap(), RECIPIENT_DELETED);
}

#[tokio::test]
async fn anexo_enviado_num_relay_descarregado_no_outro() {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let (a, _fa, ua, _b, _fb, ub) = par_federado().await;
    let (_, mut alice) = conta(&ua).await;
    let (_, mut bob) = conta(&ub).await;
    let dados: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
    let (id, token) = match alice.request(ClientMessage::BlobCreate { size: dados.len() as u64 }).await.unwrap() {
        ServerMessage::BlobCreated { blob_id, token } => (blob_id, token),
        o => panic!("{o:?}"),
    };
    let mut off = 0;
    for bloco in dados.chunks(qt_server::blobs::BLOB_CHUNK) {
        let r = alice
            .request(ClientMessage::BlobPut { blob_id: id.clone(), token: token.clone(), offset: off, data: b64.encode(bloco) })
            .await
            .unwrap();
        assert!(matches!(r, ServerMessage::BlobStatus { .. }), "{r:?}");
        off += bloco.len() as u64;
    }
    // O Bob, no relay b, ve e descarrega o anexo que esta no relay a.
    match bob.request(ClientMessage::BlobInfo { blob_id: id.clone() }).await.unwrap() {
        ServerMessage::BlobStatus { size, complete, .. } => assert_eq!((size, complete), (100_000, true)),
        o => panic!("{o:?}"),
    }
    let mut lido = Vec::new();
    while lido.len() < dados.len() {
        match bob.request(ClientMessage::BlobGet { blob_id: id.clone(), offset: lido.len() as u64 }).await.unwrap() {
            ServerMessage::BlobData { data, size, .. } => {
                assert_eq!(size, 100_000);
                lido.extend(b64.decode(data).unwrap());
            }
            o => panic!("{o:?}"),
        }
    }
    assert_eq!(lido, dados);
    // Apagar no relay b apaga no relay a.
    ack(bob.request(ClientMessage::BlobDelete { blob_id: id.clone() }).await.unwrap());
    assert!(a.store.blob_info(&id).await.is_err(), "apagado no relay de quem enviou");
    assert!(matches!(bob.request(ClientMessage::BlobGet { blob_id: id, offset: 0 }).await.unwrap(), ServerMessage::BlobNotFound));
}

#[tokio::test]
async fn copia_de_recuperacao_noutro_relay() {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let (_a, _fa, ua, _b, _fb, ub) = par_federado().await;
    let (_, mut alice) = conta(&ua).await;
    let lookup = "ab".repeat(32);
    ack(alice
        .request(ClientMessage::BackupPut {
            lookup: lookup.clone(),
            generation: "01".repeat(8),
            part: 0,
            parts: 1,
            data: b64.encode([9u8; 500]),
        })
        .await
        .unwrap());
    // Telemovel novo ligado ao relay b (ainda sem conta): encontra a copia no relay a.
    let mut novo = ligar(&ub, &SigningKeyPair::generate()).await;
    match novo.request(ClientMessage::BackupGet { lookup: lookup.clone(), part: 0 }).await.unwrap() {
        ServerMessage::BackupPart { parts, data, .. } => assert_eq!((parts, b64.decode(data).unwrap()), (1, vec![9u8; 500])),
        o => panic!("{o:?}"),
    }
    assert!(matches!(
        novo.request(ClientMessage::BackupGet { lookup: "cd".repeat(32), part: 0 }).await.unwrap(),
        ServerMessage::BackupNotFound
    ));
}

#[tokio::test]
async fn relay_do_destinatario_em_baixo_fica_na_outbox() {
    let mut a = Lado::novo("a").await;
    let b = Lado::novo("b").await;
    // O b esta na lista do a, mas nao esta ligado.
    let fa = a.arrancar(vec![b.como_par()]);
    let ua = relay(&a, Some(fa)).await;
    let (_, mut alice) = conta(&ua).await;
    recusa(alice.request(envio(&uid(&SigningKeyPair::generate()), 1)).await.unwrap(), FED_UNAVAILABLE);
}

// ---------------------------------------------------------------------------
// Fase 3: relay fechado a contas novas e pedidos do operador
// ---------------------------------------------------------------------------

#[tokio::test]
async fn relay_sem_contas_novas() {
    let (a, fa, ua, _b, _fb, _ub) = par_federado().await;
    let alice = SigningKeyPair::generate();
    ack(registar_em(&ua, &alice).await);
    // O mesmo relay "a", agora fechado a contas novas pelo operador.
    let fechado = relay_com(&a, ServerConfig { federacao: Some(fa.clone()), novas_contas: false, ..fast_cfg() }).await;
    // Conta nova: recusada (a app tenta o relay seguinte da lista).
    recusa(registar_em(&fechado, &SigningKeyPair::generate()).await, NO_NEW_ACCOUNTS);
    // A Alice ja existe: renova normalmente.
    ack(registar_em(&fechado, &alice).await);
}

#[tokio::test]
async fn relay_fechado_diz_onde_esta_a_conta() {
    let (_a, _fa, ua, b, fb, _ub) = par_federado().await;
    let alice = SigningKeyPair::generate();
    ack(registar_em(&ua, &alice).await);
    let fechado = relay_com(&b, ServerConfig { federacao: Some(fb), novas_contas: false, ..fast_cfg() }).await;
    recusa(registar_em(&fechado, &alice).await, &elsewhere(Some("a")));
}

#[tokio::test]
async fn operador_num_relay_trata_contas_do_outro() {
    let (a, fa, _ua, b, fb, ub) = par_federado().await;
    let op = SigningKeyPair::generate();
    let ua_op = relay_com(&a, ServerConfig { federacao: Some(fa), operator: Some(uid(&op)), ..fast_cfg() }).await;
    let (bs, mut bob) = conta(&ub).await;
    let bob_id = uid(&bs);
    let mut o = ligar(&ua_op, &op).await;
    // Premium para o Bob (que esta no relay b), pedido ao relay a.
    ack(o.request(ClientMessage::AdminGrantPremium { user_id: bob_id.clone(), until_ms: 4_000_000_000_000 }).await.unwrap());
    assert_eq!(b.store.premium_until(&bob_id).await.unwrap(), 4_000_000_000_000);
    match o.request(ClientMessage::AdminListPremium).await.unwrap() {
        ServerMessage::PremiumList { entries } => assert!(entries.iter().any(|e| e.user_id == bob_id)),
        x => panic!("{x:?}"),
    }
    // Contas ligadas: as dos dois relays (o Bob esta ligado ao b).
    match o.request(ClientMessage::AdminOnline).await.unwrap() {
        ServerMessage::Online { users } => assert!(users >= 1, "{users}"),
        x => panic!("{x:?}"),
    }
    // Bloqueio premium: vale nos dois relays.
    match o.request(ClientMessage::AdminPremiumEnforce { on: Some(true) }).await.unwrap() {
        ServerMessage::PremiumEnforce { on } => assert!(on),
        x => panic!("{x:?}"),
    }
    assert!(b.store.premium_enforced().await.unwrap());
    // Banir o Bob: banido no relay dele (que fecha a ligacao), e aparece na lista.
    ack(o.request(ClientMessage::AdminBan { user_id: bob_id.clone() }).await.unwrap());
    assert!(b.store.is_banned(&bob_id).await.unwrap());
    let _ = bob.fetch_messages().await;
    match o.request(ClientMessage::AdminListBans).await.unwrap() {
        ServerMessage::Bans { user_ids } => assert!(user_ids.contains(&bob_id)),
        x => panic!("{x:?}"),
    }
    ack(o.request(ClientMessage::AdminUnban { user_id: bob_id.clone() }).await.unwrap());
    assert!(!b.store.is_banned(&bob_id).await.unwrap());
    // fb fica vivo ate ao fim do teste.
    drop(fb);
}

// ---------------------------------------------------------------------------
// 0.9.69: conta nova no relay mais proximo (`qt_ffi::relay_choice`)
// ---------------------------------------------------------------------------

/// Como as apps ao criar uma conta: regista-se no primeiro relay da ordem que ainda nao foi
/// tentado; "sem contas novas" passa ao seguinte. Devolve onde ficou e os que foram tentados.
async fn registar_pela_ordem(ordem: &[String], urls: &[(String, String)], s: &SigningKeyPair) -> (String, Vec<String>) {
    let mut tentados: Vec<String> = Vec::new();
    loop {
        let nome = qt_ffi::relay_choice::relay_next_for_new_account(ordem.to_vec(), tentados.clone()).expect("algum relay aceita");
        let url = &urls.iter().find(|(n, _)| *n == nome).expect("relay da lista").1;
        match registar_em(url, s).await {
            ServerMessage::Ack => return (nome, tentados),
            ServerMessage::Error { reason } if reason == qt_ffi::relay_no_new_accounts_reason() => tentados.push(nome),
            other => panic!("resposta inesperada de {nome}: {other:?}"),
        }
    }
}

/// A ordem medida pelo nucleo: o relay da lista que nao responde fica no fim (nunca e tentado);
/// o relay fechado a contas novas e saltado e a conta fica no seguinte que aceita.
#[tokio::test]
async fn conta_nova_no_relay_mais_proximo_que_aceita() {
    use qt_ffi::relay_choice::{relay_order_for_new_account, RelayCandidate};
    assert_eq!(qt_ffi::relay_no_new_accounts_reason(), NO_NEW_ACCOUNTS, "as apps comparam com este texto");
    let (a, fa, _ua, _b, _fb, ub) = par_federado().await;
    // O relay "a" fechado a contas novas, o "b" aberto e um terceiro que nao responde.
    let fechado = relay_com(&a, ServerConfig { federacao: Some(fa.clone()), novas_contas: false, ..fast_cfg() }).await;
    let mudo = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("ws://127.0.0.1:{}", l.local_addr().unwrap().port())
    };
    let urls: Vec<(String, String)> = vec![("mudo".into(), mudo), ("a".into(), fechado), ("b".into(), ub)];
    let lista: Vec<RelayCandidate> = urls.iter().map(|(n, u)| RelayCandidate { name: n.clone(), url: u.clone() }).collect();
    let ordem = tokio::task::spawn_blocking(move || relay_order_for_new_account(lista, 2_000)).await.unwrap();
    assert_eq!(ordem.len(), 3);
    assert_eq!(ordem.last().map(String::as_str), Some("mudo"), "quem nao responde fica no fim: {ordem:?}");
    let (onde, tentados) = registar_pela_ordem(&ordem, &urls, &SigningKeyPair::generate()).await;
    assert_eq!(onde, "b");
    assert!(!tentados.contains(&"mudo".to_string()), "{tentados:?}");
    // O fechado a frente (como se fosse o mais rapido): e tentado, recusa, e a conta fica no b.
    let (onde, tentados) = registar_pela_ordem(&["a".into(), "b".into(), "mudo".into()], &urls, &SigningKeyPair::generate()).await;
    assert_eq!((onde.as_str(), tentados), ("b", vec!["a".to_string()]));
    drop(fa);
}
