//! FERRAMENTA DE TESTE (nao e a app): faz de "segundo utilizador" no PC, usando exatamente o
//! mesmo caminho que o Android (WebSocket + NoiseSession + codec do QtClient).
//!
//!   cargo run -p qt_server --example peer -- --relay ws://127.0.0.1:9443 --db peer.db
//!
//! Comandos (stdin):   /id            mostra o teu QT Public ID
//!                     /add qt1:...   define o contacto (o Public ID que a app te mostra)
//!                     qualquer texto envia essa mensagem ao contacto
//! Mensagens recebidas aparecem como   << texto
//!
//! ATENCAO: a chave do storage fica num ficheiro `<db>.key` (so para testes; a app usa o Keystore).

use futures_util::{SinkExt, StreamExt};
use qt_ffi::history::ReceiveResult;
use qt_ffi::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_tungstenite::tungstenite::Message;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

async fn exchange(ws: &mut Ws, noise: &Arc<NoiseSession>, req: Vec<u8>) -> Result<RelayResponse, String> {
    ws.send(Message::Binary(noise.encrypt(req).map_err(s)?)).await.map_err(s)?;
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
async fn rpc(qt: &Arc<QtClient>, url: &str, pin: &Option<String>, req: Vec<u8>) -> Result<RelayResponse, String> {
    let noise = qt.relay_noise_session().map_err(s)?;
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.map_err(s)?;
    while !noise.is_finished() {
        ws.send(Message::Binary(noise.write_step().map_err(s)?)).await.map_err(s)?;
        if !noise.is_finished() {
            match ws.next().await {
                Some(Ok(Message::Binary(b))) => noise.read_step(b).map_err(s)?,
                _ => return Err("handshake interrompido".into()),
            }
        }
    }
    if let Some(p) = pin {
        let got: String = noise.remote_static_public_key().map_err(s)?.iter().map(|b| format!("{b:02x}")).collect();
        if &got != p {
            return Err("chave do relay diferente da esperada (pinning)".into());
        }
    }
    let auth = qt.relay_auth_message(noise.handshake_hash().map_err(s)?).map_err(s)?;
    match exchange(&mut ws, &noise, auth).await? {
        RelayResponse::Ack => {}
        _ => return Err("autenticacao recusada".into()),
    }
    let r = exchange(&mut ws, &noise, req).await;
    let _ = ws.close(None).await;
    r
}

/// Envia tudo o que esta na outbox (mensagens e recibos) e limpa o que o relay confirmar.
async fn flush_outbox(qt: &Arc<QtClient>, url: &str, pin: &Option<String>) {
    let Ok(items) = qt.outbox() else { return };
    for it in items {
        let Ok(relay_id) = relay_id_of_contact(qt, &it.contact_id) else { continue };
        let Ok(req) = qt.relay_request_send(relay_id, it.blob) else { continue };
        if let Ok(RelayResponse::Ack) = rpc(qt, url, pin, req).await {
            let _ = qt.outbox_sent(it.outbox_id);
        }
    }
}

/// O relay identifica caixas pela chave de assinatura; guardamo-la ao definir o contacto.
fn relay_id_of_contact(_qt: &Arc<QtClient>, contact_id: &str) -> Result<String, ()> {
    RELAY_IDS.lock().unwrap().get(contact_id).cloned().ok_or(())
}

static RELAY_IDS: std::sync::LazyLock<Mutex<std::collections::HashMap<String, String>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

fn desc(r: &Result<RelayResponse, String>) -> String {
    match r {
        Ok(RelayResponse::Ack) => "Ack".into(),
        Ok(RelayResponse::Bundle { .. }) => "Bundle".into(),
        Ok(RelayResponse::BundleNotFound) => "BundleNotFound".into(),
        Ok(RelayResponse::Messages { .. }) => "Messages".into(),
        Ok(RelayResponse::Error { reason }) => format!("Error({reason})"),
        Err(e) => e.clone(),
    }
}

fn arg(name: &str, default: &str) -> String {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned()).unwrap_or_else(|| default.to_string())
}

#[tokio::main]
async fn main() {
    let url = arg("--relay", "ws://127.0.0.1:9443");
    let db = arg("--db", "peer.db");
    let pin = Some(arg("--pin", "")).filter(|p| !p.is_empty());

    // chave do storage (apenas para este utilitario de teste)
    let key_path = format!("{db}.key");
    let key: Vec<u8> = match std::fs::read_to_string(&key_path) {
        Ok(h) => (0..h.trim().len() / 2).map(|i| u8::from_str_radix(&h.trim()[2 * i..2 * i + 2], 16).unwrap_or(0)).collect(),
        Err(_) => {
            let k = qt_transport::generate_static_keypair().expect("rng").private;
            let hex: String = k.iter().map(|b| format!("{b:02x}")).collect();
            std::fs::write(&key_path, hex).expect("escrever chave");
            k
        }
    };
    let qt = QtClient::open(db, key).expect("abrir storage");
    if !qt.has_identity().expect("identidade") {
        qt.create_identity().expect("criar identidade");
    }
    let me = qt.export_public_identity().expect("identidade publica");
    println!("O teu QT Public ID:\n{}\n", me.public_id);
    match rpc(&qt, &url, &pin, qt.relay_request_register_bundle(qt.publishable_bundle().expect("bundle")).expect("req")).await {
        Ok(RelayResponse::Ack) => println!("[ligado ao relay, bundle publicado]"),
        other => println!("[erro ao publicar bundle: {}]", desc(&other)),
    }

    let contact: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    // Receber: poll de 2s
    {
        let (qt, url, pin) = (qt.clone(), url.clone(), pin.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let Ok(req) = qt.relay_request_fetch() else { continue };
                let Ok(RelayResponse::Messages { messages }) = rpc(&qt, &url, &pin, req).await else { continue };
                if messages.is_empty() {
                    continue;
                }
                let mut ids = Vec::new();
                for m in messages {
                    match qt.receive_sealed(m.ciphertext) {
                        Ok(ReceiveResult::NewMessage { message }) => {
                            println!("<< [{}…] {}", &message.contact_id[..8], message.text)
                        }
                        Ok(ReceiveResult::StatusChanged { message_ids, status, .. }) => {
                            println!("[{} mensagem(ns) agora: {:?}]", message_ids.len(), status)
                        }
                        Ok(ReceiveResult::NoChange { .. }) => {}
                        Err(QtError::Storage) => continue, // nao confirmar: o relay volta a entregar
                        Err(e) => println!("[mensagem descartada: {e}]"),
                    }
                    ids.push(m.id);
                }
                if let Ok(req) = qt.relay_request_ack(ids) {
                    let _ = rpc(&qt, &url, &pin, req).await;
                }
                // Recibos "entregue" e o que ficou por enviar.
                let _ = qt.prepare_delivery_receipts();
                flush_outbox(&qt, &url, &pin).await;
            }
        });
    }

    println!("Comandos: /id  |  /add <QT Public ID>  |  texto para enviar\n");
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line == "/id" {
            println!("{}", me.public_id);
        } else if let Some(pid) = line.strip_prefix("/add ") {
            match relay_id_of_public_id(pid.trim().to_string()) {
                Ok(rid) => {
                    let p = pid.trim().to_string();
                    RELAY_IDS.lock().unwrap().insert(p[4..68].to_string(), rid);
                    *contact.lock().unwrap() = Some(p);
                    println!("[contacto definido]");
                }
                Err(e) => println!("[Public ID invalido: {e}]"),
            }
        } else {
            let Some(pid) = contact.lock().unwrap().clone() else {
                println!("[define primeiro o contacto com /add <Public ID>]");
                continue;
            };
            let relay_id = relay_id_of_public_id(pid.clone()).unwrap_or_default();
            let cid: String = pid[4..68].to_string();
            if !qt.has_session(cid.clone()).unwrap_or(false) {
                let req = qt.relay_request_fetch_bundle(relay_id.clone()).expect("req");
                match rpc(&qt, &url, &pin, req).await {
                    Ok(RelayResponse::Bundle { bytes }) => {
                        let r = peer_bundle_from_relay_bytes(bytes).and_then(|b| qt.start_session(pid.clone(), b));
                        if let Err(e) = r {
                            println!("[nao foi possivel iniciar sessao: {e}]");
                            continue;
                        }
                    }
                    other => {
                        println!("[o contacto ainda nao publicou o bundle: {}]", desc(&other));
                        continue;
                    }
                }
            }
            match qt.send_text(cid, line) {
                Ok(_) => {
                    flush_outbox(&qt, &url, &pin).await;
                    println!(">> enviada");
                }
                Err(e) => println!("[erro a cifrar: {e}]"),
            }
        }
    }
}
