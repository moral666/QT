//! FERRAMENTA DO EMISSOR DOS BADGES (so no computador do operador; nao e a app nem o servidor).
//!
//! Passo 1, uma vez, na pasta do repo: criar a chave do emissor (fica num ficheiro, NUNCA no git
//! nem no servidor). A chave publica e escrita em `ISSUER_PUBLIC_KEY` (core/src/badge.rs);
//! so as apps compiladas com ela aceitam os badges.
//!
//! ```text
//! cargo run --release -p qt_server --example badge -- chave --key ~/Documentos/badge-emissor.key
//! ```
//!
//! Passo 2, dar um badge (founder, cofounder, dev, tester, contributor) ou tirar (nenhum):
//!
//! ```text
//! cargo run --release -p qt_server --example badge -- dar --key ~/Documentos/badge-emissor.key \
//!     --user qt1:... --badge tester
//! ```
//!
//! A ferramenta tem uma identidade propria (`--db`, por omissao badge-emissor.db) e envia o
//! certificado a pessoa pelo relay, cifrado, como uma mensagem normal. A app guarda-o e
//! envia-o aos seus contactos. O relay nao sabe que e um badge e nao guarda nada.

use futures_util::{SinkExt, StreamExt};
use qt_core::badge;
use qt_ffi::*;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message;

const RELAY: &str = "wss://relay.sealerapp.com";
const PIN: &str = "f018d46b67d2437571cefc86827dec0dc3b0fd586bd22c664d3839f6a1c67f1a";

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

async fn exchange(
    ws: &mut Ws,
    noise: &Arc<NoiseSession>,
    req: Vec<u8>,
) -> Result<RelayResponse, String> {
    ws.send(Message::Binary(noise.encrypt(req).map_err(s)?))
        .await
        .map_err(s)?;
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(15), ws.next()).await {
            Err(_) => return Err("timeout".into()),
            Ok(None) => return Err("ligacao fechada".into()),
            Ok(Some(Err(e))) => return Err(s(e)),
            Ok(Some(Ok(Message::Binary(b)))) => {
                return relay_parse_response(noise.decrypt(b).map_err(s)?).map_err(s)
            }
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
            Ok(Some(Ok(other))) => return Err(format!("frame inesperado: {other:?}")),
        }
    }
}

/// Uma ligacao nova por pedido (simples; a app Android mantem a ligacao e faz reconnect).
async fn rpc(
    qt: &Arc<QtClient>,
    url: &str,
    pin: &Option<String>,
    req: Vec<u8>,
) -> Result<RelayResponse, String> {
    let noise = qt.relay_noise_session().map_err(s)?;
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.map_err(s)?;
    while !noise.is_finished() {
        ws.send(Message::Binary(noise.write_step().map_err(s)?))
            .await
            .map_err(s)?;
        if !noise.is_finished() {
            match ws.next().await {
                Some(Ok(Message::Binary(b))) => noise.read_step(b).map_err(s)?,
                _ => return Err("handshake interrompido".into()),
            }
        }
    }
    if let Some(p) = pin {
        let got: String = noise
            .remote_static_public_key()
            .map_err(s)?
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if &got != p {
            return Err("chave do relay diferente da esperada (pinning)".into());
        }
    }
    let auth = qt
        .relay_auth_message(noise.handshake_hash().map_err(s)?)
        .map_err(s)?;
    match exchange(&mut ws, &noise, auth).await? {
        RelayResponse::Ack => {}
        _ => return Err("autenticacao recusada".into()),
    }
    let r = exchange(&mut ws, &noise, req).await;
    let _ = ws.close(None).await;
    r
}

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(h: &str) -> Option<[u8; 32]> {
    let h = h.trim();
    if h.len() != 64 {
        return None;
    }
    let v: Option<Vec<u8>> = (0..32)
        .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok())
        .collect();
    v?.try_into().ok()
}

fn die(msg: &str) -> ! {
    eprintln!("ERRO: {msg}");
    std::process::exit(1)
}

fn badge_num(name: &str) -> u8 {
    match name.to_lowercase().replace(['-', '_'], "").as_str() {
        "nenhum" | "none" | "tirar" => badge::BADGE_NONE,
        "founder" => badge::BADGE_FOUNDER,
        "cofounder" => badge::BADGE_COFOUNDER,
        "dev" => badge::BADGE_DEV,
        "tester" => badge::BADGE_TESTER,
        "contributor" => badge::BADGE_CONTRIBUTOR,
        _ => die("badge desconhecido (founder, cofounder, dev, tester, contributor ou nenhum)"),
    }
}

fn write_private(path: &str, contents: &str) {
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    o.mode(0o600);
    let mut f = o
        .open(path)
        .unwrap_or_else(|e| die(&format!("nao foi possivel criar {path}: {e}")));
    f.write_all(contents.as_bytes())
        .unwrap_or_else(|e| die(&e.to_string()));
}

fn read_seed(path: &str) -> [u8; 32] {
    let h = std::fs::read_to_string(path)
        .unwrap_or_else(|e| die(&format!("nao foi possivel ler {path}: {e}")));
    unhex32(&h).unwrap_or_else(|| die("ficheiro da chave invalido"))
}

fn cmd_chave() {
    let path = arg("--key").unwrap_or_else(|| die("falta --key <ficheiro>"));
    if std::path::Path::new(&path).exists() {
        die("o ficheiro ja existe: nao se sobrescreve a chave do emissor");
    }
    let seed = qt_transport::generate_static_keypair()
        .expect("rng")
        .private;
    let seed: [u8; 32] = seed[..32].try_into().expect("32 bytes");
    write_private(&path, &hex(&seed));
    let public = badge::issuer_public(&seed);
    let bytes: Vec<String> = public.iter().map(|b| format!("0x{b:02x}")).collect();
    println!("Chave do emissor guardada em {path} (guarda uma copia na pen).\n");
    println!("Chave publica: {}\n", hex(&public));
    let line = format!(
        "pub const ISSUER_PUBLIC_KEY: [u8; 32] = [{}];",
        bytes.join(", ")
    );
    // Corrido na pasta do repo: escreve logo a chave publica no nucleo (so se ainda estiver a zeros).
    let core = "core/src/badge.rs";
    let zeros = "pub const ISSUER_PUBLIC_KEY: [u8; 32] = [0u8; 32];";
    match std::fs::read_to_string(core) {
        Ok(src) if src.contains(zeros) => {
            std::fs::write(core, src.replace(zeros, &line)).unwrap_or_else(|e| die(&e.to_string()));
            println!("Chave publica escrita em {core}. Falta o commit e compilar as apps.");
        }
        _ => println!("Em {core}:\n{line}"),
    }
}

async fn cmd_dar() {
    let key_path = arg("--key").unwrap_or_else(|| die("falta --key <ficheiro>"));
    let user = arg("--user").unwrap_or_else(|| die("falta --user qt1:..."));
    let num = badge_num(&arg("--badge").unwrap_or_else(|| die("falta --badge")));
    let url = arg("--relay").unwrap_or_else(|| RELAY.into());
    let pin = Some(arg("--pin").unwrap_or_else(|| PIN.into())).filter(|p| !p.is_empty());
    let db = arg("--db").unwrap_or_else(|| "badge-emissor.db".into());

    let seed = read_seed(&key_path);
    if badge::issuer_public(&seed) != badge::ISSUER_PUBLIC_KEY {
        die("esta chave nao e a que esta em ISSUER_PUBLIC_KEY (core/src/badge.rs): as apps vao recusar o badge");
    }
    let relay_id =
        relay_id_of_public_id(user.clone()).unwrap_or_else(|e| die(&format!("ID invalido: {e}")));
    let cid = user[4..68].to_lowercase();
    let subject = unhex32(&cid).unwrap_or_else(|| die("ID invalido"));

    // Identidade propria da ferramenta (chave da base de dados ao lado, so neste PC).
    let db_key_path = format!("{db}.key");
    let db_key: Vec<u8> = match std::fs::read_to_string(&db_key_path) {
        Ok(h) => unhex32(&h)
            .unwrap_or_else(|| die("chave da base de dados invalida"))
            .to_vec(),
        Err(_) => {
            let k = qt_transport::generate_static_keypair()
                .expect("rng")
                .private;
            write_private(&db_key_path, &hex(&k));
            k
        }
    };
    let qt = QtClient::open(db, db_key).unwrap_or_else(|e| die(&format!("base de dados: {e}")));
    if !qt.has_identity().unwrap_or(false) {
        qt.create_identity().unwrap_or_else(|e| die(&e.to_string()));
    }
    // Publica o bundle (cria a conta no relay na primeira vez).
    let req = qt
        .relay_request_register_bundle(qt.publishable_bundle().expect("bundle"))
        .expect("pedido");
    match rpc(&qt, &url, &pin, req).await {
        Ok(RelayResponse::Ack) => {}
        Ok(_) => die("o relay recusou o registo"),
        Err(e) => die(&format!("relay: {e}")),
    }
    if !qt.has_session(cid.clone()).unwrap_or(false) {
        let req = qt
            .relay_request_fetch_bundle(relay_id.clone())
            .expect("pedido");
        match rpc(&qt, &url, &pin, req).await {
            Ok(RelayResponse::Bundle { bytes }) => {
                peer_bundle_from_relay_bytes(bytes)
                    .and_then(|b| qt.start_session(user.clone(), b))
                    .unwrap_or_else(|e| die(&format!("sessao: {e}")));
            }
            _ => die("essa pessoa ainda nao publicou o bundle (abriu a app?)"),
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("relogio")
        .as_millis() as i64;
    let cert = badge::issue(&seed, &subject, num, now);
    qt.badge_deliver(cid.clone(), cert)
        .unwrap_or_else(|e| die(&format!("certificado: {e}")));
    let mut sent = 0;
    for it in qt.outbox().unwrap_or_default() {
        let req = qt
            .relay_request_send(relay_id.clone(), it.blob)
            .expect("pedido");
        match rpc(&qt, &url, &pin, req).await {
            Ok(RelayResponse::Ack) => {
                let _ = qt.outbox_sent(it.outbox_id);
                sent += 1;
            }
            other => eprintln!(
                "[nao enviado: {}]",
                match other {
                    Err(e) => e,
                    Ok(_) => "recusado".into(),
                }
            ),
        }
    }
    if sent == 0 {
        die("nada foi entregue ao relay; tenta outra vez");
    }
    println!(
        "Feito: {} para {}… (chega quando a app abrir).",
        arg("--badge").unwrap_or_default(),
        &cid[..12]
    );
}

#[tokio::main]
async fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("chave") => cmd_chave(),
        Some("dar") => cmd_dar().await,
        _ => die("uso: badge chave --key <ficheiro> | badge dar --key <ficheiro> --user qt1:... --badge <nome>"),
    }
}
