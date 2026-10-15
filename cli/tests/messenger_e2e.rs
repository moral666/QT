//! Testes de ponta a ponta do `messenger` (o binario real), contra um relay
//! real (WebSocket + Noise + Redis) a correr neste processo:
//!   * duas identidades do `messenger` trocam mensagens e recibos;
//!   * o `messenger` fala com um cliente que usa o nucleo EXATAMENTE como as
//!     apps (QtClient + NoiseSession), nos dois sentidos;
//!   * pinning errado e recusado; passphrase errada nao abre nada.
//!
//! Precisa de Redis em 127.0.0.1:6379 (como os testes do servidor e o CI).

use futures_util::{SinkExt, StreamExt};
use qt_ffi::history::ReceiveResult;
use qt_ffi::*;
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message;

const BIN: &str = env!("CARGO_BIN_EXE_messenger");

struct Relay {
    url: String,
    pin: String,
}

/// Arranca um relay real numa porta livre (prefixo Redis proprio por teste).
async fn start_relay(prefix: &str) -> Relay {
    let keys = qt_transport::generate_static_keypair().unwrap();
    let pin: String = keys.public.iter().map(|b| format!("{b:02x}")).collect();
    let store = Arc::new(
        qt_server::Store::with_prefix("redis://127.0.0.1:6379", &format!("cli-e2e-{prefix}-{}", std::process::id()))
            .unwrap(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let private = keys.private.clone();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { continue };
            let (store, private) = (store.clone(), private.clone());
            tokio::spawn(async move {
                let _ = qt_server::handle_connection(tcp, store, private).await;
            });
        }
    });
    Relay { url, pin }
}

fn tmpdb(name: &str) -> String {
    let p = std::env::temp_dir().join(format!("qt_cli_e2e_{name}_{}.sqlite", std::process::id()));
    let s = p.to_string_lossy().to_string();
    for f in [s.clone(), format!("{s}.salt"), format!("{s}.contacts")] {
        let _ = std::fs::remove_file(f);
    }
    s
}

/// Corre o `messenger` com a passphrase no stdin (como num script).
async fn run(db: &str, pass: &str, relay: Option<&Relay>, args: &[&str]) -> Output {
    let (db, pass) = (db.to_string(), pass.to_string());
    let mut cmd_args: Vec<String> = vec!["--passphrase-stdin".into(), "--db".into(), db];
    if let Some(r) = relay {
        cmd_args.extend(["--server".into(), r.url.clone(), "--pin".into(), r.pin.clone()]);
    }
    cmd_args.extend(args.iter().map(|a| a.to_string()));
    tokio::task::spawn_blocking(move || {
        let mut child = Command::new(BIN)
            .args(&cmd_args)
            .env_remove("QT_RELAY_URL")
            .env_remove("QT_RELAY_PIN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(child.stdin.take().unwrap(), "{pass}").unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap()
}

fn out(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn ok(o: Output) -> String {
    assert!(o.status.success(), "o messenger falhou:\n{}", out(&o));
    out(&o)
}

fn public_id_from(text: &str) -> String {
    text.split_whitespace().find(|w| w.starts_with("qt1:")).expect("sem QT Public ID na saida").to_string()
}

// ------------------------------------------------- cliente "app" (QtClient)

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// O mesmo caminho que o RelayLink das apps: WebSocket + NoiseSession + auth.
struct AppLink {
    ws: Ws,
    noise: Arc<NoiseSession>,
}

async fn next_bin(ws: &mut Ws) -> Vec<u8> {
    loop {
        match ws.next().await.unwrap().unwrap() {
            Message::Binary(b) => return b,
            _ => continue,
        }
    }
}

impl AppLink {
    async fn connect(qt: &Arc<QtClient>, relay: &Relay) -> AppLink {
        let noise = qt.relay_noise_session().unwrap();
        let (mut ws, _) = tokio_tungstenite::connect_async(&relay.url).await.unwrap();
        while !noise.is_finished() {
            ws.send(Message::Binary(noise.write_step().unwrap())).await.unwrap();
            if !noise.is_finished() {
                noise.read_step(next_bin(&mut ws).await).unwrap();
            }
        }
        let mut l = AppLink { ws, noise };
        let auth = qt.relay_auth_message(l.noise.handshake_hash().unwrap()).unwrap();
        assert!(matches!(l.req(auth).await, RelayResponse::Ack));
        l
    }
    async fn req(&mut self, r: Vec<u8>) -> RelayResponse {
        self.ws.send(Message::Binary(self.noise.encrypt(r).unwrap())).await.unwrap();
        let f = next_bin(&mut self.ws).await;
        relay_parse_response(self.noise.decrypt(f).unwrap()).unwrap()
    }
}

// =================================================================== testes

#[tokio::test(flavor = "multi_thread")]
async fn dois_terminais_trocam_mensagens_e_recibos() {
    let relay = start_relay("cli2").await;
    let (ana, rui) = (tmpdb("ana"), tmpdb("rui"));
    let pass_a = "passphrase da Ana 123";
    let pass_r = "passphrase do Rui 456";

    let id_a = public_id_from(&ok(run(&ana, pass_a, None, &["identity"]).await));
    let id_r = public_id_from(&ok(run(&rui, pass_r, None, &["identity"]).await));
    assert_eq!(public_id_from(&ok(run(&ana, pass_a, None, &["id"]).await)), id_a);

    // O Rui publica as chaves; a Ana adiciona-o e escreve.
    ok(run(&rui, pass_r, Some(&relay), &["register"]).await);
    ok(run(&ana, pass_a, None, &["add", "Rui", &id_r]).await);
    let s = ok(run(&ana, pass_a, Some(&relay), &["send", "--to", "Rui", "--message", "ola Rui"]).await);
    assert!(s.contains("enviada"), "{s}");

    // O Rui recebe (ainda nao tem a Ana nos contactos: aparece como desconhecido).
    let r = ok(run(&rui, pass_r, Some(&relay), &["receive"]).await);
    assert!(r.contains("ola Rui") && r.contains("desconhecido"), "{r}");

    // O Rui adiciona a Ana e responde; os recibos chegam a Ana.
    ok(run(&rui, pass_r, None, &["add", "Ana", &id_a]).await);
    ok(run(&rui, pass_r, Some(&relay), &["send", "--to", "Ana", "--message", "ola Ana"]).await);
    let a = ok(run(&ana, pass_a, Some(&relay), &["receive"]).await);
    assert!(a.contains("Rui: ola Ana"), "{a}");
    assert!(a.contains("vista") || a.contains("entregue"), "a Ana recebe o recibo: {a}");

    // Historico cifrado no disco, com estados.
    let h = ok(run(&ana, pass_a, None, &["history", "Rui"]).await);
    assert!(h.contains("eu: ola Rui") && h.contains("Rui: ola Ana"), "{h}");
    let raw = std::fs::read(&ana).unwrap();
    assert!(!raw.windows(7).any(|w| w == b"ola Rui"), "o historico nao fica em claro no ficheiro");
    let rawc = std::fs::read(format!("{ana}.contacts")).unwrap();
    assert!(!rawc.windows(3).any(|w| w == b"Rui"), "os contactos nao ficam em claro");

    // Passphrase errada: nao abre.
    let bad = run(&ana, "passphrase errada!!", None, &["id"]).await;
    assert!(!bad.status.success() && out(&bad).contains("passphrase errada"), "{}", out(&bad));

    // Apagar contacto: some da lista e do historico.
    ok(run(&ana, pass_a, None, &["remove", "Rui"]).await);
    let c = ok(run(&ana, pass_a, None, &["contacts"]).await);
    assert!(!c.contains("Rui"), "{c}");
}

#[tokio::test(flavor = "multi_thread")]
async fn terminal_e_app_falam_nos_dois_sentidos() {
    let relay = start_relay("app").await;
    let cli_db = tmpdb("terminal");
    let pass = "passphrase do terminal 1";
    let id_cli = public_id_from(&ok(run(&cli_db, pass, None, &["identity"]).await));

    // A "app": QtClient com uma chave de 32 bytes (nas apps vem do Keystore).
    let app_db = tmpdb("app");
    let app = QtClient::open(app_db, vec![0x5a; 32]).unwrap();
    let me = app.create_identity().unwrap();
    let mut link = AppLink::connect(&app, &relay).await;
    let r = link.req(app.relay_request_register_bundle(app.publishable_bundle().unwrap()).unwrap()).await;
    assert!(matches!(r, RelayResponse::Ack));

    // Terminal -> app.
    ok(run(&cli_db, pass, None, &["add", "Telemovel", &me.public_id]).await);
    ok(run(&cli_db, pass, Some(&relay), &["send", "--to", "Telemovel", "--message", "do PC para o telemovel"]).await);
    let msgs = match link.req(app.relay_request_fetch().unwrap()).await {
        RelayResponse::Messages { messages } => messages,
        _ => panic!("sem mensagens"),
    };
    let mut got = Vec::new();
    let mut ids = Vec::new();
    for m in msgs {
        if let ReceiveResult::NewMessage { message } = app.receive_sealed(m.ciphertext).unwrap() {
            got.push(message.text);
        }
        ids.push(m.id);
    }
    assert_eq!(got, vec!["do PC para o telemovel".to_string()]);
    assert!(matches!(link.req(app.relay_request_ack(ids).unwrap()).await, RelayResponse::Ack));

    // App -> terminal (a app responde na sessao que o terminal abriu).
    let cid = id_cli[4..68].to_string();
    app.prepare_delivery_receipts().unwrap();
    app.send_text(cid, "do telemovel para o PC".into()).unwrap();
    let relay_id = relay_id_of_public_id(id_cli.clone()).unwrap();
    for item in app.outbox().unwrap() {
        let r = link.req(app.relay_request_send(relay_id.clone(), item.blob).unwrap()).await;
        assert!(matches!(r, RelayResponse::Ack));
        app.outbox_sent(item.outbox_id).unwrap();
    }
    let t = ok(run(&cli_db, pass, Some(&relay), &["receive"]).await);
    assert!(t.contains("Telemovel: do telemovel para o PC"), "{t}");
    assert!(t.contains("entregue"), "o terminal recebe o recibo 'entregue' da app: {t}");
}

#[tokio::test(flavor = "multi_thread")]
async fn pin_errado_e_recusado_e_base_existente_nao_e_reescrita() {
    let relay = start_relay("pin").await;
    let db = tmpdb("pin");
    let pass = "uma passphrase longa";
    ok(run(&db, pass, None, &["identity"]).await);
    let wrong = Relay { url: relay.url.clone(), pin: "00".repeat(32) };
    let o = run(&db, pass, Some(&wrong), &["register"]).await;
    assert!(!o.status.success() && out(&o).contains("intermediario"), "{}", out(&o));
    // Criar outra vez por cima da mesma base de dados e recusado.
    let again = run(&db, pass, None, &["identity"]).await;
    assert!(!again.status.success() && out(&again).contains("ja existe"), "{}", out(&again));
    // Passphrase curta demais para uma base nova.
    let short = run(&tmpdb("curta"), "curta", None, &["identity"]).await;
    assert!(!short.status.success(), "{}", out(&short));
}
