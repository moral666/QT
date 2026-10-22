//! Client A <-> Relay <-> Client B, com transporte real (WebSocket + Noise),
//! E2EE real (QtClient: PQXDH + Double Ratchet + Sealed Sender) e Redis real.

mod common;
use common::*;

use futures_util::{SinkExt, StreamExt};
use qt_ffi::*;
use qt_server::client::{RelayClient, RelayConfig};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

fn tmp(n: &str) -> String {
    // Contador global: cada chamada devolve um caminho UNICO (os testes correm
    // em paralelo no mesmo processo e varios usam o mesmo nome, ex. "alice").
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_two_{}_{}_{}.sqlite", n, std::process::id(), id);
    let _ = std::fs::remove_file(&p);
    p
}
const KA: [u8; 32] = [0xA1; 32];
const KB: [u8; 32] = [0xB2; 32];

struct User {
    path: String,
    key: [u8; 32],
    qt: Arc<QtClient>,
    id: PublicIdentity,
}
fn user(name: &str, key: [u8; 32]) -> User {
    let path = tmp(name);
    let qt = QtClient::open(path.clone(), key.to_vec()).unwrap();
    let id = qt.create_identity().unwrap();
    User { path, key, qt, id }
}
impl User {
    fn restart(&mut self) {
        self.qt = QtClient::open(self.path.clone(), self.key.to_vec()).unwrap();
    }
    fn contact_id(&self) -> String {
        self.id.identity_public.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

// ----------------------------------------------------------------------
// Caminho 1: cliente Rust (RelayClient com reconnect/timeouts) + QtClient
// ----------------------------------------------------------------------

/// Para o RelayClient, a assinatura e a chave Noise tem de vir do QtClient;
/// no teste usamos chaves proprias do RelayClient so para a LIGACAO, mas a
/// caixa de correio (user_id) e derivada da chave de assinatura. Para ter a
/// MESMA caixa que o QtClient, ligamos pelo caminho "Android" no teste 2.
/// Aqui testamos o transporte com a caixa de correio do RelayClient.
fn mailbox_signing(seed: u8) -> [u8; 32] {
    [seed; 32]
}

#[tokio::test]
async fn conversa_a_b_pelo_relay_com_quedas_e_reinicios() {
    let mut relay = TestRelay::start(fast_cfg()).await;
    let (mut alice, mut bob) = (user("alice", KA), user("bob", KB));
    // Caixas de correio de teste (ligadas pelo RelayClient).
    let (box_a, box_b) = (mailbox_signing(1), mailbox_signing(2));
    let noise = |_: u8| qt_transport::generate_static_keypair().unwrap().private;
    let cfg = RelayConfig { max_retries: 3, backoff_base: Duration::from_millis(50), request_timeout: Duration::from_secs(3), ..RelayConfig::default() };
    let mut ra = RelayClient::connect(&relay.url, noise(1), box_a, cfg.clone()).await.unwrap();
    let mut rb = RelayClient::connect(&relay.url, noise(2), box_b, cfg.clone()).await.unwrap();

    // Bob publica o bundle; Alice descobre-o pelo relay e inicia PQXDH.
    // Esta caixa de correio de teste NAO usa a chave de identidade de Bob, por isso o
    // relay (corretamente) recusaria o registo pela rede; publica-se direto no store.
    // O registo autenticado com a identidade real e testado mais abaixo (`relay_request_register_bundle`).
    qt_server::Store::with_prefix("redis://127.0.0.1:6379", &relay.prefix)
        .unwrap()
        .register_bundle(rb.user_id().to_string(), bob.qt.publishable_bundle().unwrap())
        .await
        .unwrap();
    // A caixa da Alice tambem precisa de registo para poder enviar.
    registar(&relay, ra.user_id()).await;
    let bytes = ra.fetch_bundle(rb.user_id()).await.unwrap().expect("bundle de Bob");
    let peer = peer_bundle_from_relay_bytes(bytes).unwrap();
    let bob_cid = alice.qt.start_session(bob.id.public_id.clone(), peer).unwrap();
    let alice_cid = alice.contact_id();

    // 1) Alice -> Bob (selado)
    let blob = alice.qt.encrypt_sealed(bob_cid.clone(), b"ola Bob, via relay".to_vec()).unwrap();
    ra.send_message(rb.user_id(), vec![], blob).await.unwrap();

    // O que o relay guarda (Redis REAL): sem plaintext nem identidades.
    let redis = redis::Client::open("redis://127.0.0.1:6379").unwrap();
    let mut rc = redis.get_connection().unwrap();
    let raw: Vec<Vec<u8>> = redis::cmd("LRANGE").arg(format!("{}:queue:{}", relay.prefix, rb.user_id())).arg(0).arg(-1).query(&mut rc).unwrap();
    assert_eq!(raw.len(), 1);
    let stored = &raw[0];
    let has = |needle: &[u8]| stored.windows(needle.len()).any(|w| w == needle);
    assert!(!has(b"ola Bob"), "plaintext visivel no Redis");
    assert!(!has(&alice.id.identity_public), "identidade do remetente visivel no Redis");
    assert!(!has(&alice.id.signing_public));

    // Bob recebe, decifra, persiste e SO ENTAO confirma.
    let msgs = rb.fetch_messages().await.unwrap();
    assert_eq!(msgs.len(), 1);
    let got = bob.qt.decrypt_sealed(msgs[0].ciphertext.clone()).unwrap();
    assert_eq!(got.plaintext, b"ola Bob, via relay");
    assert_eq!(got.contact_id, alice_cid);

    // 2) Entrega duplicada (ack nunca chegou): Bob re-recebe, o QtClient rejeita o replay, Bob confirma.
    let again = rb.fetch_messages().await.unwrap();
    assert_eq!(again.len(), 1, "sem ack o relay re-entrega");
    assert!(matches!(bob.qt.decrypt_sealed(again[0].ciphertext.clone()), Err(QtError::Replay)));
    rb.ack(again.iter().map(|m| m.id).collect()).await.unwrap();
    assert!(rb.fetch_messages().await.unwrap().is_empty());

    // 3) Bob reinicia (processo morto): estado criptografico restaura-se do storage.
    bob.restart();
    let reply = bob.qt.encrypt_sealed(alice_cid.clone(), b"ola Alice".to_vec()).unwrap();
    rb.send_message(ra.user_id(), vec![], reply).await.unwrap();

    // 4) O RELAY cai e volta enquanto ha mensagens em fila e clientes ligados.
    relay.kill();
    let blob2 = alice.qt.encrypt_sealed(bob_cid.clone(), b"enviada com o relay em baixo".to_vec()).unwrap();
    let relay_url = relay.url.clone();
    let _ = relay_url;
    // O envio falha (indisponivel) - a mensagem fica com a app, o estado criptografico ja avancou e esta persistido.
    assert!(ra.send_message(rb.user_id(), vec![], blob2.clone()).await.is_err());
    alice.restart(); // e a app de Alice tambem reinicia
    relay.restart(fast_cfg()).await;
    // Reenvio do MESMO envelope (idempotente: o destinatario deduplica).
    ra.send_message(rb.user_id(), vec![], blob2.clone()).await.unwrap();
    ra.send_message(rb.user_id(), vec![], blob2).await.unwrap(); // duplicado na rede

    let m = ra.fetch_messages().await.unwrap();
    assert_eq!(m.len(), 1);
    let got = alice.qt.decrypt_sealed(m[0].ciphertext.clone()).unwrap();
    assert_eq!(got.plaintext, b"ola Alice", "mensagem que estava em fila sobreviveu ao reinicio do relay");
    ra.ack(vec![m[0].id]).await.unwrap();

    let m = rb.fetch_messages().await.unwrap();
    assert_eq!(m.len(), 2, "original + duplicado");
    let first = bob.qt.decrypt_sealed(m[0].ciphertext.clone()).unwrap();
    assert_eq!(first.plaintext, b"enviada com o relay em baixo");
    assert!(matches!(bob.qt.decrypt_sealed(m[1].ciphertext.clone()), Err(QtError::Replay)));
    rb.ack(m.iter().map(|x| x.id).collect()).await.unwrap();

    // 5) Conversa continua normalmente depois de tudo.
    for i in 0..5u8 {
        let b = alice.qt.encrypt_sealed(bob_cid.clone(), vec![i; 30]).unwrap();
        ra.send_message(rb.user_id(), vec![], b).await.unwrap();
        let m = rb.fetch_messages().await.unwrap();
        assert_eq!(bob.qt.decrypt_sealed(m[0].ciphertext.clone()).unwrap().plaintext, vec![i; 30]);
        rb.ack(vec![m[0].id]).await.unwrap();
    }
}

// ----------------------------------------------------------------------
// Caminho 2: exatamente o que o Android faz - WebSocket (OkHttp) + NoiseSession
// + codec do QtClient. Nenhuma chave privada sai do Rust.
// ----------------------------------------------------------------------

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct AndroidLink {
    ws: Ws,
    noise: Arc<NoiseSession>,
}

async fn android_connect(qt: &Arc<QtClient>, url: &str) -> AndroidLink {
    let noise = qt.relay_noise_session().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    while !noise.is_finished() {
        let out = noise.write_step().unwrap();
        ws.send(Message::Binary(out)).await.unwrap();
        if !noise.is_finished() {
            let Message::Binary(b) = ws.next().await.unwrap().unwrap() else { panic!("frame") };
            noise.read_step(b).unwrap();
        }
    }
    let mut link = AndroidLink { ws, noise };
    let auth = qt.relay_auth_message(link.noise.handshake_hash().unwrap()).unwrap();
    let resp = link.rpc(auth).await;
    assert!(matches!(resp, RelayResponse::Ack), "auth: {}", match &resp { RelayResponse::Error { reason } => reason.clone(), _ => "?".into() });
    // Como a app ao ligar-se: publica o bundle (so contas registadas escrevem no relay).
    let reg = link.rpc(qt.relay_request_register_bundle(qt.publishable_bundle().unwrap()).unwrap()).await;
    assert!(matches!(reg, RelayResponse::Ack), "registo: {}", match &reg { RelayResponse::Error { reason } => reason.clone(), _ => "?".into() });
    link
}

impl AndroidLink {
    async fn rpc(&mut self, request: Vec<u8>) -> RelayResponse {
        let ct = self.noise.encrypt(request).unwrap();
        self.ws.send(Message::Binary(ct)).await.unwrap();
        loop {
            match self.ws.next().await.unwrap().unwrap() {
                Message::Binary(b) => return relay_parse_response(self.noise.decrypt(b).unwrap()).unwrap(),
                Message::Ping(_) | Message::Pong(_) => continue,
                other => panic!("frame inesperado {other:?}"),
            }
        }
    }
}

#[tokio::test]
async fn caminho_android_ws_noise_codec_end_to_end() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (alice, mut bob) = (user("alice2", KA), user("bob2", KB));
    let mut la = android_connect(&alice.qt, &relay.url).await;
    let mut lb = android_connect(&bob.qt, &relay.url).await;
    assert_eq!(alice.qt.relay_user_id().unwrap(), alice.id.relay_id);

    // Bob publica o bundle.
    let r = lb.rpc(bob.qt.relay_request_register_bundle(bob.qt.publishable_bundle().unwrap()).unwrap()).await;
    assert!(matches!(r, RelayResponse::Ack));

    // Alice adiciona o contacto pelo QT Public ID de Bob.
    let bob_relay = relay_id_of_public_id(bob.id.public_id.clone()).unwrap();
    assert_eq!(bob_relay, bob.id.relay_id);
    let RelayResponse::Bundle { bytes } = la.rpc(alice.qt.relay_request_fetch_bundle(bob_relay.clone()).unwrap()).await else { panic!("bundle") };
    let cid = alice.qt.start_session(bob.id.public_id.clone(), peer_bundle_from_relay_bytes(bytes).unwrap()).unwrap();

    // Relay malicioso entrega o bundle de outra pessoa -> start_session recusa.
    let eve = user("eve2", [0xEE; 32]);
    let evil = peer_bundle_from_relay_bytes(eve.qt.publishable_bundle().unwrap()).unwrap();
    assert!(matches!(alice.qt.start_session(bob.id.public_id.clone(), evil), Err(QtError::InvalidBundle)));

    // Alice -> Bob
    let blob = alice.qt.encrypt_sealed(cid.clone(), "ola, mensagem 📨".as_bytes().to_vec()).unwrap();
    assert!(matches!(la.rpc(alice.qt.relay_request_send(bob_relay.clone(), blob).unwrap()).await, RelayResponse::Ack));

    let RelayResponse::Messages { messages } = lb.rpc(bob.qt.relay_request_fetch().unwrap()).await else { panic!("msgs") };
    assert_eq!(messages.len(), 1);
    let dec = bob.qt.decrypt_sealed(messages[0].ciphertext.clone()).unwrap();
    assert_eq!(dec.plaintext, "ola, mensagem 📨".as_bytes());
    let ids: Vec<u64> = messages.iter().map(|m| m.id).collect();
    assert!(matches!(lb.rpc(bob.qt.relay_request_ack(ids).unwrap()).await, RelayResponse::Ack));

    // Bob reinicia, reabre o storage e liga-se de novo (nova sessao Noise, mesma chave estatica persistida)
    let static_before = bob.qt.relay_noise_session().unwrap();
    drop(static_before);
    bob.restart();
    drop(lb);
    let mut lb = android_connect(&bob.qt, &relay.url).await;
    let reply = bob.qt.encrypt_sealed(dec.contact_id.clone(), b"resposta".to_vec()).unwrap();
    let alice_relay = alice.id.relay_id.clone();
    assert!(matches!(lb.rpc(bob.qt.relay_request_send(alice_relay, reply).unwrap()).await, RelayResponse::Ack));
    let RelayResponse::Messages { messages } = la.rpc(alice.qt.relay_request_fetch().unwrap()).await else { panic!() };
    assert_eq!(alice.qt.decrypt_sealed(messages[0].ciphertext.clone()).unwrap().plaintext, b"resposta");

    // Bob nao consegue ler a caixa de Alice (a assinatura e dele).
    let spoof = format!(r#"{{"FetchMessages":{{"user_id":"{}"}}}}"#, alice.id.relay_id).into_bytes();
    assert!(matches!(lb.rpc(spoof).await, RelayResponse::Error { .. }));

    // Entradas invalidas do codec nao entram em panic.
    assert!(relay_parse_response(vec![]).is_err());
    assert!(relay_parse_response(b"{\"X\":1}".to_vec()).is_err());
    assert!(peer_bundle_from_relay_bytes(vec![1, 2, 3]).is_err());
    let mut old_version = bob.qt.publishable_bundle().unwrap();
    old_version[0] = 1;
    assert!(peer_bundle_from_relay_bytes(old_version).is_err(), "bundle v1 (assinatura antiga) nao e aceite");
    assert!(alice.qt.relay_auth_message(vec![0; 5]).is_err());
    assert!(relay_id_of_public_id("lixo".into()).is_err());
}

#[tokio::test]
async fn android_pinning_chave_estatica_do_relay_exposta() {
    let relay = TestRelay::start(fast_cfg()).await;
    let a = user("alice3", KA);
    let l = android_connect(&a.qt, &relay.url).await;
    // A app pode comparar com a chave de relay que embebeu (pinning).
    assert_eq!(l.noise.remote_static_public_key().unwrap(), relay.static_public);
}

/// Entrega imediata, pelo caminho exato das apps: o Bob tem uma segunda ligacao so
/// para esperar (`relay_request_wait`); a mensagem da Alice chega-lhe no momento em
/// que entra no relay, ja cifrada de ponta a ponta, e e confirmada pela ligacao principal.
#[tokio::test]
async fn caminho_android_entrega_imediata_por_ligacao_de_espera() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (alice, bob) = (user("alice_wait", KA), user("bob_wait", KB));
    let mut la = android_connect(&alice.qt, &relay.url).await;
    let mut lb = android_connect(&bob.qt, &relay.url).await;
    let mut lb_wait = android_connect(&bob.qt, &relay.url).await;

    assert!(matches!(lb.rpc(bob.qt.relay_request_register_bundle(bob.qt.publishable_bundle().unwrap()).unwrap()).await, RelayResponse::Ack));
    let bob_relay = bob.id.relay_id.clone();
    let RelayResponse::Bundle { bytes } = la.rpc(alice.qt.relay_request_fetch_bundle(bob_relay.clone()).unwrap()).await else { panic!("bundle") };
    let cid = alice.qt.start_session(bob.id.public_id.clone(), peer_bundle_from_relay_bytes(bytes).unwrap()).unwrap();

    // O Bob fica a espera (fila vazia).
    let wait_req = bob.qt.relay_request_wait(20).unwrap();
    let espera = tokio::spawn(async move {
        let t = std::time::Instant::now();
        let r = lb_wait.rpc(wait_req).await;
        (r, t.elapsed(), lb_wait)
    });
    tokio::time::sleep(Duration::from_millis(400)).await;

    let enviada = std::time::Instant::now();
    let blob = alice.qt.encrypt_sealed(cid, "chegou já".as_bytes().to_vec()).unwrap();
    assert!(matches!(la.rpc(alice.qt.relay_request_send(bob_relay, blob).unwrap()).await, RelayResponse::Ack));

    let (r, esperou, mut lb_wait) = espera.await.unwrap();
    let entrega = enviada.elapsed();
    let RelayResponse::Messages { messages } = r else { panic!("esperava Messages") };
    assert_eq!(messages.len(), 1);
    assert!(esperou >= Duration::from_millis(300), "devia ter esperado: {esperou:?}");
    assert!(entrega < Duration::from_secs(1), "entrega devia ser imediata: {entrega:?}");
    assert_eq!(bob.qt.decrypt_sealed(messages[0].ciphertext.clone()).unwrap().plaintext, "chegou já".as_bytes());
    assert!(matches!(lb.rpc(bob.qt.relay_request_ack(vec![messages[0].id]).unwrap()).await, RelayResponse::Ack));

    // Sem mais nada na fila, a espera seguinte acaba vazia ao fim do tempo pedido.
    let RelayResponse::Messages { messages } = lb_wait.rpc(bob.qt.relay_request_wait(1).unwrap()).await else { panic!() };
    assert!(messages.is_empty());
}
