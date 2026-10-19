//! A campainha (FCM) a tocar de verdade, com um servidor FALSO no lugar da Google
//! (`QT_FCM_BASE_URL` e o `token_uri` da conta de servico apontam para ele). Prova:
//!   * app fechada (ninguem a espera): toca, uma vez, com o token certo e sem nada
//!     da mensagem;
//!   * logo a seguir (limite de frequencia): nao toca outra vez;
//!   * app aberta (`WaitMessages` pendente): nao toca;
//!   * envio `silent` (recibo, reacao, presenca): entregue, mas nao toca;
//!   * token que a Google diz que ja nao existe: e apagado do Redis.
//! Precisa de um Redis em 127.0.0.1:6379 e do `openssl` (gera uma chave RSA de teste).

mod common;
use common::*;

use qt_core::primitives::SigningKeyPair;
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::protocol::*;
use qt_server::{ServerConfig, Store};
use qt_transport::generate_static_keypair;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn uid(s: &SigningKeyPair) -> String {
    user_id_from_signing_public(&s.verifying_key.to_bytes())
}

async fn client(relay: &TestRelay, s: &SigningKeyPair) -> RelayClient {
    let n = generate_static_keypair().unwrap();
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    RelayClient::connect(&relay.url, n.private, s.to_bytes(), cfg).await.unwrap()
}

/// Pedidos recebidos pelo servidor falso: (caminho, corpo).
type Log = Arc<Mutex<Vec<(String, String)>>>;

/// Servidor HTTP minimo: /token devolve um token de acesso; /v1/... responde 200, ou
/// 404 UNREGISTERED se o token do telemovel comecar por "morto".
async fn fake_google() -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let l2 = log.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { return };
            let log = l2.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                loop {
                    // Le um pedido inteiro (cabecalhos + Content-Length) de cada vez.
                    let mut tmp = [0u8; 4096];
                    let head_end = loop {
                        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                        match s.read(&mut tmp).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                    let len = head
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                        .unwrap_or(0);
                    while buf.len() < head_end + len {
                        match s.read(&mut tmp).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        }
                    }
                    let body = String::from_utf8_lossy(&buf[head_end..head_end + len]).to_string();
                    buf.drain(..head_end + len);
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    let (status, reply) = if path == "/token" {
                        ("200 OK", r#"{"access_token":"acesso-falso","expires_in":3600}"#.to_string())
                    } else if body.contains("\"token\":\"morto") {
                        ("404 Not Found", r#"{"error":{"status":"NOT_FOUND","details":[{"errorCode":"UNREGISTERED"}]}}"#.to_string())
                    } else {
                        ("200 OK", r#"{"name":"projects/p/messages/1"}"#.to_string())
                    };
                    log.lock().unwrap().push((path, body));
                    let resp = format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{reply}", reply.len());
                    if s.write_all(resp.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (base, log)
}

fn sends(log: &Log) -> Vec<String> {
    log.lock().unwrap().iter().filter(|(p, _)| p.contains("messages:send")).map(|(_, b)| b.clone()).collect()
}

async fn wait_for(log: &Log, n: usize) -> bool {
    for _ in 0..60 {
        if sends(log).len() >= n {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Liga as notificacoes neste processo (o relay de teste corre aqui), contra o servidor falso.
fn enable_push(base: &str) {
    let dir = std::env::temp_dir().join(format!("qt_fcm_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pem = dir.join("k.pem");
    let ok = std::process::Command::new("openssl")
        .args(["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out"])
        .arg(&pem)
        .status()
        .expect("este teste precisa do openssl")
        .success();
    assert!(ok);
    let sa = serde_json::json!({
        "project_id": "projeto-teste",
        "client_email": "relay@projeto-teste.iam.gserviceaccount.com",
        "private_key": std::fs::read_to_string(&pem).unwrap(),
        "token_uri": format!("{base}/token"),
    });
    let json = dir.join("sa.json");
    std::fs::write(&json, sa.to_string()).unwrap();
    std::env::set_var("QT_FCM_BASE_URL", base);
    std::env::set_var("QT_FCM_CREDENTIALS", &json);
    assert!(qt_server::push::init_from_env().unwrap());
    assert!(qt_server::push::enabled());
}

/// Tudo num so teste: as notificacoes sao globais no processo do relay.
#[tokio::test]
async fn toca_so_com_a_app_fechada_e_sem_nada_da_mensagem() {
    let (base, log) = fake_google().await;
    enable_push(&base);
    let relay = TestRelay::start(ServerConfig::default()).await;
    let store = Store::with_prefix("redis://127.0.0.1:6379", &relay.prefix).unwrap();

    let (alice, bob, carol, dave) =
        (SigningKeyPair::generate(), SigningKeyPair::generate(), SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut a = client(&relay, &alice).await;
    let mut b = client(&relay, &bob).await;
    let mut c = client(&relay, &carol).await;
    let mut d = client(&relay, &dave).await;
    let erin = SigningKeyPair::generate();
    let mut e = client(&relay, &erin).await;
    for (cl, who, tok) in [(&mut b, &bob, "tok-bob:APA91"), (&mut c, &carol, "tok-carol"), (&mut d, &dave, "morto-dave"), (&mut e, &erin, "tok-erin")] {
        let r = cl.request(ClientMessage::RegisterPushToken { user_id: uid(who), token: tok.into() }).await.unwrap();
        assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    }

    // 1) Bob com a app fechada: toca, com o token dele, sem conteudo nem remetente.
    a.send_message(&uid(&bob), b"remetente-secreto".to_vec(), b"conteudo-secreto".to_vec()).await.unwrap();
    assert!(wait_for(&log, 1).await, "devia ter tocado");
    let first = sends(&log)[0].clone();
    assert!(first.contains("tok-bob:APA91"));
    for leak in ["secreto", &uid(&bob), &uid(&alice), "notification"] {
        assert!(!first.contains(leak), "a Google nao pode ver {leak:?}: {first}");
    }

    // 2) Logo a seguir: limite de frequencia, nao toca outra vez.
    a.send_message(&uid(&bob), vec![1], vec![2]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(sends(&log).len(), 1, "segunda campainha em menos de 10 s");

    // 3) Carol com a app aberta (a espera): a mensagem chega-lhe, sem campainha.
    let carol_id = uid(&carol);
    let cid = carol_id.clone();
    let waiting = tokio::spawn(async move {
        c.request_with_timeout(ClientMessage::WaitMessages { user_id: cid, timeout_secs: 10 }, Duration::from_secs(15)).await
    });
    for _ in 0..100 {
        if qt_server::waiters::is_waiting(&carol_id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(qt_server::waiters::is_waiting(&carol_id), "a Carol devia estar a espera");
    a.send_message(&carol_id, vec![1], vec![2]).await.unwrap();
    let got = waiting.await.unwrap().unwrap();
    assert!(matches!(got, ServerMessage::Messages { ref messages } if !messages.is_empty()), "{got:?}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(sends(&log).len(), 1, "com a app aberta nao toca");

    // 3b) Erin com a app fechada, mas o envio e `silent` (recibo/presenca): fica na fila, sem campainha.
    let r = a
        .request(ClientMessage::SendMessage { to: uid(&erin), sealed_from: vec![], ciphertext: vec![9], silent: true, wake: None })
        .await
        .unwrap();
    assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(sends(&log).len(), 1, "um envio silent nao toca");
    let r = e.request(ClientMessage::FetchMessages { user_id: uid(&erin) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Messages { ref messages } if messages.len() == 1), "{r:?}");

    // 3c) Chamada para o Bob (app fechada): campainha marcada, mesmo logo depois da do 1).
    let call = |silent| ClientMessage::SendMessage {
        to: uid(&bob),
        sealed_from: vec![],
        ciphertext: vec![7],
        silent,
        wake: Some(qt_server::protocol::WakeKind::Call),
    };
    let r = a.request(call(false)).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    assert!(wait_for(&log, 2).await, "a chamada devia tocar");
    let ring = sends(&log)[1].clone();
    assert!(ring.contains("\"qt\":\"call\"") && ring.contains("tok-bob:APA91"), "{ring}");
    for leak in ["secreto", &uid(&bob), &uid(&alice), "notification"] {
        assert!(!ring.contains(leak), "a Google nao pode ver {leak:?}: {ring}");
    }
    // Cancelada antes de atender: "parar de tocar".
    let end = ClientMessage::SendMessage {
        to: uid(&bob),
        sealed_from: vec![],
        ciphertext: vec![8],
        silent: false,
        wake: Some(qt_server::protocol::WakeKind::CallEnd),
    };
    let r = a.request(end.clone()).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    assert!(wait_for(&log, 3).await, "o cancelamento devia tocar");
    assert!(sends(&log)[2].contains("\"qt\":\"call_end\""));
    // Um segundo "parar" (sem chamada a tocar) nao toca.
    a.request(end).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(sends(&log).len(), 3, "parar sem chamada nao toca");

    // 4) Token que ja nao existe (app desinstalada): apagado do Redis.
    a.send_message(&uid(&dave), vec![1], vec![2]).await.unwrap();
    assert!(wait_for(&log, 4).await);
    for _ in 0..40 {
        if store.get_push_token(&uid(&dave)).await.unwrap().is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(store.get_push_token(&uid(&dave)).await.unwrap(), None);
    // O token do Bob continua la.
    assert!(store.get_push_token(&uid(&bob)).await.unwrap().is_some());
}
