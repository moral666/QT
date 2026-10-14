//! Relay: autenticacao, isolamento entre utilizadores, entrega at-least-once,
//! limites, timeouts, frames invalidos, reconnect e pinning.

mod common;
use common::*;

use futures_util::SinkExt;
use qt_core::primitives::SigningKeyPair;
use qt_server::client::{RelayClient, RelayConfig, RelayError};
use qt_server::protocol::*;
use qt_server::ServerConfig;
use qt_transport::{generate_static_keypair, ws_transport};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

fn uid(s: &SigningKeyPair) -> String {
    user_id_from_signing_public(&s.verifying_key.to_bytes())
}
fn quick() -> RelayConfig {
    RelayConfig {
        connect_timeout: Duration::from_secs(2),
        request_timeout: Duration::from_millis(800),
        max_retries: 0,
        backoff_base: Duration::from_millis(50),
        expected_server_static: None,
    }
}
async fn client(relay: &TestRelay, s: &SigningKeyPair, cfg: RelayConfig) -> RelayClient {
    let n = generate_static_keypair().unwrap();
    RelayClient::connect(&relay.url, n.private, s.to_bytes(), cfg).await.unwrap()
}
async fn raw(url: &str) -> (qt_transport::ws_transport::ClientWsStream, qt_transport::NoiseTransport) {
    let k = generate_static_keypair().unwrap();
    ws_transport::client_connect(url, &k.private).await.unwrap()
}
async fn send(ws: &mut ws_transport::ClientWsStream, n: &mut qt_transport::NoiseTransport, m: &ClientMessage) {
    ws_transport::send_encrypted(ws, n, &serialize_client_message(m)).await.unwrap();
}
async fn recv(ws: &mut ws_transport::ClientWsStream, n: &mut qt_transport::NoiseTransport) -> Result<ServerMessage, ()> {
    let b = ws_transport::receive_encrypted_timeout(ws, n, Duration::from_secs(2)).await.map_err(|_| ())?;
    deserialize_server_message(&b).map_err(|_| ())
}
async fn authenticate(ws: &mut ws_transport::ClientWsStream, n: &mut qt_transport::NoiseTransport, s: &SigningKeyPair) {
    let m = auth_msg(s, &n.handshake_hash().to_vec());
    send(ws, n, &m).await;
}
fn auth_msg(s: &SigningKeyPair, hash: &[u8]) -> ClientMessage {
    let mut m = AUTH_CONTEXT.to_vec();
    m.extend_from_slice(hash);
    ClientMessage::Authenticate {
        signing_public: s.verifying_key.to_bytes().to_vec(),
        signature: s.sign(&m).to_bytes().to_vec(),
    }
}

#[tokio::test]
async fn sem_autenticacao_o_relay_recusa_e_fecha() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (mut ws, mut n) = raw(&relay.url).await;
    send(&mut ws, &mut n, &ClientMessage::FetchMessages { user_id: "0".repeat(64) }).await;
    assert!(matches!(recv(&mut ws, &mut n).await, Ok(ServerMessage::Error { .. })));
    // a ligacao foi encerrada
    send(&mut ws, &mut n, &ClientMessage::FetchPreKeyBundle { user_id: "0".repeat(64) }).await;
    assert!(recv(&mut ws, &mut n).await.is_err());
}

#[tokio::test]
async fn assinatura_de_autenticacao_nao_serve_noutra_sessao() {
    let relay = TestRelay::start(fast_cfg()).await;
    let victim = SigningKeyPair::generate();
    // Atacante apanha a assinatura valida de uma sessao...
    let (mut ws1, n1) = raw(&relay.url).await;
    let stolen = auth_msg(&victim, n1.handshake_hash());
    drop(ws1.close(None).await);
    // ...e tenta reutiliza-la numa sessao NOVA (hash de handshake diferente).
    let (mut ws2, mut n2) = raw(&relay.url).await;
    send(&mut ws2, &mut n2, &stolen).await;
    assert!(matches!(recv(&mut ws2, &mut n2).await, Ok(ServerMessage::Error { .. })));
    // Chave de assinatura errada / lixo
    let (mut ws3, mut n3) = raw(&relay.url).await;
    send(&mut ws3, &mut n3, &ClientMessage::Authenticate { signing_public: vec![1; 5], signature: vec![2; 7] }).await;
    assert!(matches!(recv(&mut ws3, &mut n3).await, Ok(ServerMessage::Error { .. })));
}

#[tokio::test]
async fn utilizadores_nao_acedem_a_filas_ou_bundles_de_outros() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (alice, bob, mallory) = (SigningKeyPair::generate(), SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut a = client(&relay, &alice, quick()).await;
    let mut b = client(&relay, &bob, quick()).await;
    let mut m = client(&relay, &mallory, quick()).await;

    a.send_message(&uid(&bob), vec![], b"envelope-1".to_vec()).await.unwrap();
    let bundle_bob = common::valid_bundle(&bob, 1);
    b.register_bundle(bundle_bob.clone()).await.unwrap();

    // Mallory: ler / apagar a fila do Bob, sobrescrever o bundle do Bob.
    let r = m.request(ClientMessage::FetchMessages { user_id: uid(&bob) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { .. }));
    let r = m.request(ClientMessage::AckMessages { user_id: uid(&bob), ids: vec![1, 2, 3] }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { .. }));
    let r = m.request(ClientMessage::RegisterPreKeyBundle { user_id: uid(&bob), bundle_bytes: common::valid_bundle(&mallory, 9) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { .. }), "nao se escreve no slot do Bob");
    // Mallory regista no SEU slot um bundle que reclama a chave do Bob (e assinado por Bob): recusado.
    let r = m.request(ClientMessage::RegisterPreKeyBundle { user_id: uid(&mallory), bundle_bytes: bundle_bob.clone() }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { .. }), "bundle de outra identidade");
    // Mallory assina o seu bundle mas adultera a identidade X25519 depois: assinatura invalida.
    let mut adulterado = common::valid_bundle(&mallory, 9);
    adulterado[1] ^= 0xFF;
    let r = m.request(ClientMessage::RegisterPreKeyBundle { user_id: uid(&mallory), bundle_bytes: adulterado }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { .. }), "assinatura nao cobre a identidade");
    // Lixo arbitrario no proprio slot tambem e recusado.
    let r = m.request(ClientMessage::RegisterPreKeyBundle { user_id: uid(&mallory), bundle_bytes: b"falso".to_vec() }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { .. }));

    assert_eq!(a.fetch_bundle(&uid(&bob)).await.unwrap().unwrap(), bundle_bob);
    let msgs = b.fetch_messages().await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].ciphertext, b"envelope-1");
    // erros nao revelam detalhes internos
    if let ServerMessage::Error { reason } = m.request(ClientMessage::FetchMessages { user_id: uid(&bob) }).await.unwrap() {
        assert!(!reason.to_lowercase().contains("redis"));
    }
}

#[tokio::test]
async fn entrega_at_least_once_so_apaga_apos_ack() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (alice, bob) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut a = client(&relay, &alice, quick()).await;
    for i in 0..3u8 {
        a.send_message(&uid(&bob), vec![], vec![i; 20]).await.unwrap();
    }
    let mut b = client(&relay, &bob, quick()).await;
    let first = b.fetch_messages().await.unwrap();
    assert_eq!(first.len(), 3);
    // Ligacao cai ANTES do ack: nada se perde.
    b.close().await;
    let mut b2 = client(&relay, &bob, quick()).await;
    let again = b2.fetch_messages().await.unwrap();
    assert_eq!(again.len(), 3, "sem ack, o relay re-entrega");
    assert_eq!(again.iter().map(|m| m.id).collect::<Vec<_>>(), first.iter().map(|m| m.id).collect::<Vec<_>>());
    // Ack parcial
    b2.ack(vec![first[0].id, first[1].id]).await.unwrap();
    let rest = b2.fetch_messages().await.unwrap();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].id, first[2].id);
    // Ack de ids inexistentes/repetido e inofensivo
    b2.ack(vec![first[0].id, 9_999_999]).await.unwrap();
    b2.ack(vec![first[2].id]).await.unwrap();
    assert!(b2.fetch_messages().await.unwrap().is_empty());
}

#[tokio::test]
async fn limites_e_validacao_de_entrada() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (alice, bob) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut a = client(&relay, &alice, quick()).await;
    let r = |x: Result<(), RelayError>| matches!(x, Err(RelayError::Rejected(_)));
    assert!(r(a.send_message(&uid(&bob), vec![], vec![0; 14_001]).await), "ciphertext grande");
    assert!(r(a.send_message(&uid(&bob), vec![], vec![]).await), "ciphertext vazio");
    assert!(r(a.send_message(&uid(&bob), vec![0; 1025], vec![1]).await), "sealed_from grande");
    assert!(r(a.send_message("bob", vec![], vec![1]).await), "destinatario nao-hex");
    assert!(r(a.send_message(&"A".repeat(64), vec![], vec![1]).await), "maiusculas");
    assert!(r(a.send_message(&"a".repeat(65), vec![], vec![1]).await), "tamanho");
    assert!(r(a.register_bundle(vec![0; 24_001]).await));
    assert!(r(a.register_bundle(vec![]).await));
    assert!(r(a.ack((0..201).collect()).await));
    assert!(matches!(a.fetch_bundle("../../etc").await, Err(RelayError::Rejected(_))));
    // lixo (JSON invalido) nao derruba a sessao
    let (mut ws, mut n) = raw(&relay.url).await;
    authenticate(&mut ws, &mut n, &alice).await;
    assert!(matches!(recv(&mut ws, &mut n).await, Ok(ServerMessage::Ack)));
    ws_transport::send_encrypted(&mut ws, &mut n, b"isto nao e json").await.unwrap();
    assert!(matches!(recv(&mut ws, &mut n).await, Ok(ServerMessage::Error { .. })));
    send(&mut ws, &mut n, &ClientMessage::FetchMessages { user_id: uid(&alice) }).await;
    assert!(matches!(recv(&mut ws, &mut n).await, Ok(ServerMessage::Messages { .. })));
}

#[tokio::test]
async fn fila_cheia_e_rejeitada_sem_derrubar_o_relay() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (alice, bob) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut a = client(&relay, &alice, RelayConfig { request_timeout: Duration::from_secs(5), ..quick() }).await;
    let mut last = Ok(());
    for _ in 0..1005 {
        last = a.send_message(&uid(&bob), vec![], vec![1; 10]).await;
        if last.is_err() {
            break;
        }
    }
    assert!(matches!(last, Err(RelayError::Rejected(ref s)) if s.contains("cheia")));
}

async fn frame_invalido_encerra(tamper: impl Fn(&mut Vec<u8>)) {
    let relay = TestRelay::start(fast_cfg()).await;
    let s = SigningKeyPair::generate();
    let (mut ws, mut n) = raw(&relay.url).await;
    authenticate(&mut ws, &mut n, &s).await;
    assert!(matches!(recv(&mut ws, &mut n).await, Ok(ServerMessage::Ack)));
    let mut ct = n.encrypt(&serialize_client_message(&ClientMessage::FetchMessages { user_id: uid(&s) })).unwrap();
    tamper(&mut ct);
    ws.send(Message::Binary(ct)).await.unwrap();
    // Sessao invalida -> o servidor encerra (sem resposta).
    assert!(recv(&mut ws, &mut n).await.is_err());
}

#[tokio::test]
async fn frame_noise_corrompido_encerra_a_sessao() {
    frame_invalido_encerra(|ct| ct[3] ^= 0x01).await;
}

#[tokio::test]
async fn frame_noise_repetido_replay_encerra_a_sessao() {
    let relay = TestRelay::start(fast_cfg()).await;
    let s = SigningKeyPair::generate();
    let (mut ws, mut n) = raw(&relay.url).await;
    authenticate(&mut ws, &mut n, &s).await;
    assert!(matches!(recv(&mut ws, &mut n).await, Ok(ServerMessage::Ack)));
    let ct = n.encrypt(&serialize_client_message(&ClientMessage::FetchMessages { user_id: uid(&s) })).unwrap();
    ws.send(Message::Binary(ct.clone())).await.unwrap();
    assert!(matches!(recv(&mut ws, &mut n).await, Ok(ServerMessage::Messages { .. })));
    ws.send(Message::Binary(ct)).await.unwrap(); // o MESMO frame outra vez
    assert!(recv(&mut ws, &mut n).await.is_err(), "frame repetido tem de encerrar a sessao");
}

#[tokio::test]
async fn frame_noise_fora_de_ordem_encerra_a_sessao() {
    let relay = TestRelay::start(fast_cfg()).await;
    let s = SigningKeyPair::generate();
    let (mut ws, mut n) = raw(&relay.url).await;
    authenticate(&mut ws, &mut n, &s).await;
    assert!(matches!(recv(&mut ws, &mut n).await, Ok(ServerMessage::Ack)));
    let m = serialize_client_message(&ClientMessage::FetchMessages { user_id: uid(&s) });
    let c1 = n.encrypt(&m).unwrap();
    let c2 = n.encrypt(&m).unwrap();
    ws.send(Message::Binary(c2)).await.unwrap(); // chega o 2.o primeiro
    assert!(recv(&mut ws, &mut n).await.is_err());
    let _ = c1;
}

#[tokio::test]
async fn ping_do_websocket_nao_derruba_a_sessao() {
    let relay = TestRelay::start(fast_cfg()).await;
    let s = SigningKeyPair::generate();
    let (mut ws, mut n) = raw(&relay.url).await;
    authenticate(&mut ws, &mut n, &s).await;
    assert!(matches!(recv(&mut ws, &mut n).await, Ok(ServerMessage::Ack)));
    ws.send(Message::Ping(vec![1, 2, 3])).await.unwrap();
    send(&mut ws, &mut n, &ClientMessage::FetchMessages { user_id: uid(&s) }).await;
    assert!(matches!(recv(&mut ws, &mut n).await, Ok(ServerMessage::Messages { .. })));
}

#[tokio::test]
async fn timeouts_de_handshake_autenticacao_e_inatividade() {
    let cfg = ServerConfig {
        handshake_timeout: Duration::from_millis(300),
        auth_timeout: Duration::from_millis(300),
        idle_timeout: Duration::from_millis(500),
    };
    let relay = TestRelay::start(cfg).await;
    // 1) TCP aberto, nada enviado: o relay fecha no timeout do handshake.
    use tokio::io::AsyncReadExt;
    let mut tcp = tokio::net::TcpStream::connect(("127.0.0.1", relay.port)).await.unwrap();
    let mut buf = [0u8; 16];
    let r = tokio::time::timeout(Duration::from_secs(3), tcp.read(&mut buf)).await.expect("o relay devia fechar");
    assert!(matches!(r, Ok(0) | Err(_)));
    // 2) Handshake feito, sem autenticar: fecha no timeout de auth.
    let (mut ws, mut n) = raw(&relay.url).await;
    assert!(recv(&mut ws, &mut n).await.is_err());
    // 3) Autenticado mas inativo: fecha por inatividade.
    let s = SigningKeyPair::generate();
    let (mut ws, mut n) = raw(&relay.url).await;
    authenticate(&mut ws, &mut n, &s).await;
    assert!(matches!(recv(&mut ws, &mut n).await, Ok(ServerMessage::Ack)));
    tokio::time::sleep(Duration::from_millis(900)).await;
    send(&mut ws, &mut n, &ClientMessage::FetchMessages { user_id: uid(&s) }).await;
    assert!(recv(&mut ws, &mut n).await.is_err(), "ligacao inativa deve ter sido fechada");
}

#[tokio::test]
async fn cliente_reconecta_sozinho_depois_de_inatividade() {
    let cfg = ServerConfig { idle_timeout: Duration::from_millis(400), ..fast_cfg() };
    let relay = TestRelay::start(cfg).await;
    let s = SigningKeyPair::generate();
    let mut c = client(&relay, &s, RelayConfig { max_retries: 3, ..quick() }).await;
    tokio::time::sleep(Duration::from_millis(900)).await; // o relay fecha a ligacao
    assert!(c.fetch_messages().await.unwrap().is_empty(), "reconnect transparente");
    assert!(c.is_connected());
}

#[tokio::test]
async fn relay_indisponivel_e_depois_recuperado() {
    let mut relay = TestRelay::start(fast_cfg()).await;
    let (alice, bob) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut a = client(&relay, &alice, RelayConfig { max_retries: 2, ..quick() }).await;
    a.send_message(&uid(&bob), vec![], b"antes da queda".to_vec()).await.unwrap();

    relay.kill(); // perda total de rede / relay em baixo
    let t = std::time::Instant::now();
    let e = a.send_message(&uid(&bob), vec![], b"durante".to_vec()).await.unwrap_err();
    assert!(matches!(e, RelayError::Unavailable | RelayError::Timeout), "{e:?}");
    assert!(t.elapsed() < Duration::from_secs(10), "falha em tempo limitado (sem bloquear)");
    assert!(!a.is_connected());

    relay.restart(fast_cfg()).await; // a rede volta
    a.send_message(&uid(&bob), vec![], b"depois".to_vec()).await.unwrap();
    let mut b = client(&relay, &bob, quick()).await;
    let msgs = b.fetch_messages().await.unwrap();
    let texts: Vec<&[u8]> = msgs.iter().map(|m| m.ciphertext.as_slice()).collect();
    assert_eq!(texts, vec![b"antes da queda".as_slice(), b"depois".as_slice()], "filas Redis sobrevivem ao reinicio");
}

#[tokio::test]
async fn pinning_da_chave_do_relay() {
    let relay = TestRelay::start(fast_cfg()).await;
    let s = SigningKeyPair::generate();
    let n = generate_static_keypair().unwrap();
    // pin correto
    let ok = RelayClient::connect(&relay.url, n.private.clone(), s.to_bytes(),
        RelayConfig { expected_server_static: Some(relay.static_public.clone()), ..quick() }).await;
    assert!(ok.is_ok());
    // pin errado (relay impostor / MITM): SessionInvalid, sem retries
    let bad = RelayClient::connect(&relay.url, n.private.clone(), s.to_bytes(),
        RelayConfig { expected_server_static: Some(vec![9; 32]), max_retries: 5, ..quick() }).await;
    assert!(matches!(bad, Err(RelayError::SessionInvalid)));
}

#[tokio::test]
async fn timeout_de_pedido_quando_o_relay_nao_responde() {
    // Relay "fantasma": faz handshake e autentica, mas nunca responde a pedidos.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let keys = generate_static_keypair().unwrap();
    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let k = keys.private.clone();
            tokio::spawn(async move {
                let (mut ws, mut n) = ws_transport::server_accept(tcp, &k).await.unwrap();
                let _ = ws_transport::receive_encrypted(&mut ws, &mut n).await; // auth
                let _ = ws_transport::send_encrypted(&mut ws, &mut n, &serialize_server_message(&ServerMessage::Ack)).await;
                tokio::time::sleep(Duration::from_secs(30)).await;
            });
        }
    });
    let s = SigningKeyPair::generate();
    let n = generate_static_keypair().unwrap();
    let mut c = RelayClient::connect(&format!("ws://127.0.0.1:{port}"), n.private, s.to_bytes(),
        RelayConfig { request_timeout: Duration::from_millis(300), max_retries: 1, backoff_base: Duration::from_millis(10), ..quick() }).await.unwrap();
    let t = std::time::Instant::now();
    assert!(matches!(c.fetch_messages().await, Err(RelayError::Timeout)));
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn encerramento_limpo_e_reuso() {
    let relay = TestRelay::start(fast_cfg()).await;
    let s = SigningKeyPair::generate();
    let mut c = client(&relay, &s, quick()).await;
    c.close().await;
    assert!(!c.is_connected());
    assert!(c.fetch_messages().await.is_ok(), "pode voltar a usar-se apos close");
}

#[tokio::test]
async fn cliente_recusa_ws_sem_tls_para_destino_remoto_sem_repetir() {
    let n = generate_static_keypair().unwrap();
    let s = SigningKeyPair::generate();
    let cfg = RelayConfig { max_retries: 5, backoff_base: Duration::from_secs(5), ..quick() };
    let t = std::time::Instant::now();
    let r = RelayClient::connect("ws://192.0.2.1:9443", n.private, s.to_bytes(), cfg).await;
    assert!(matches!(r, Err(RelayError::InsecureUrl)), "ws:// remoto tem de ser recusado");
    assert!(t.elapsed() < Duration::from_secs(1), "erro permanente: sem backoff/repeticoes");
}
