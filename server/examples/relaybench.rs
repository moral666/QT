//! MEDICAO DO RELAY (mensagens por segundo e latencia), sem a criptografia do
//! cliente: o relay so ve bytes opacos, por isso aqui os "envelopes" sao bytes
//! aleatorios do tamanho pedido. Mede o que custa ao relay + Redis guardar,
//! entregar e confirmar mensagens.
//!
//!   cargo run --release -p qt_server --example relaybench -- \
//!       --relay ws://127.0.0.1:9443 --pairs 50 --size 4096 --secs 30 --rate 0
//!
//! `--pairs N`: N remetentes, cada um a escrever para o seu destinatario (2N ligacoes
//! autenticadas, cada uma com a sua identidade, como as apps).
//! `--rate R`: mensagens por segundo por remetente (0 = o mais depressa possivel,
//! com um pedido de cada vez, como a app).
//! `--poll-ms P`: intervalo entre pedidos de mensagens quando a fila esta vazia
//! (a app usa 3000; aqui 20 por omissao para medir o relay, nao o intervalo).
//! `--warmup W`: segundos iniciais que nao contam para as medicoes.
//! `--wait-secs S`: o destinatario usa `WaitMessages` (entrega imediata) em vez de
//! perguntar de `--poll-ms` em `--poll-ms` (0 = desligado, o comportamento antigo).
//!
//! Escreve: envios/s, entregas/s, tempo de resposta a um envio (p50/p95/p99),
//! latencia de ponta a ponta (envio ate o destinatario o ter), erros, mensagens
//! perdidas ou repetidas.

use qt_core::primitives::SigningKeyPair;
use qt_server::protocol::*;
use qt_transport::{generate_static_keypair, ws_transport, NoiseTransport};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Ws = ws_transport::ClientWsStream;

fn arg(name: &str, default: &str) -> String {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned()).unwrap_or_else(|| default.to_string())
}

#[derive(Default)]
struct Counters {
    sent: AtomicU64,
    send_errors: AtomicU64,
    delivered: AtomicU64,
    duplicates: AtomicU64,
    fetches: AtomicU64,
    bytes_out: AtomicU64,
}

struct Samples {
    rtt_us: Mutex<Vec<u64>>,
    e2e_us: Mutex<Vec<u64>>,
}

/// WebSocket + handshake Noise XX. (Nao usa `ws_transport::client_connect` porque
/// esse, compilado em release, so aceita wss://; aqui o relay e local, sem TLS.)
async fn connect(url: &str) -> Result<(Ws, NoiseTransport), String> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let k = generate_static_keypair().map_err(|e| e.to_string())?;
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.map_err(|e| e.to_string())?;
    let mut hs = qt_transport::NoiseHandshake::new_initiator(&k.private).map_err(|e| e.to_string())?;
    while !hs.is_finished() {
        ws.send(Message::Binary(hs.write_step().map_err(|e| e.to_string())?)).await.map_err(|e| e.to_string())?;
        if !hs.is_finished() {
            match ws.next().await {
                Some(Ok(Message::Binary(b))) => hs.read_step(&b).map_err(|e| e.to_string())?,
                _ => return Err("handshake interrompido".into()),
            }
        }
    }
    Ok((ws, hs.into_transport().map_err(|e| e.to_string())?))
}

async fn login(url: &str, s: &SigningKeyPair) -> Result<(Ws, NoiseTransport), String> {
    let (mut ws, mut n) = connect(url).await?;
    let mut m = AUTH_CONTEXT.to_vec();
    m.extend_from_slice(n.handshake_hash());
    let auth = ClientMessage::Authenticate {
        signing_public: s.verifying_key.to_bytes().to_vec(),
        signature: s.sign(&m).to_bytes().to_vec(),
    };
    match req(&mut ws, &mut n, &auth).await? {
        ServerMessage::Ack => Ok((ws, n)),
        _ => Err("autenticacao recusada".into()),
    }
}

async fn req(ws: &mut Ws, n: &mut NoiseTransport, m: &ClientMessage) -> Result<ServerMessage, String> {
    ws_transport::send_encrypted(ws, n, &serialize_client_message(m)).await.map_err(|e| e.to_string())?;
    let b = ws_transport::receive_encrypted_timeout(ws, n, Duration::from_secs(30)).await.map_err(|e| e.to_string())?;
    deserialize_server_message(&b).map_err(|e| e.to_string())
}

fn pct(v: &mut [u64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_unstable();
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[i] as f64 / 1000.0
}

#[tokio::main]
async fn main() {
    let url = arg("--relay", "ws://127.0.0.1:9443");
    let pairs: usize = arg("--pairs", "20").parse().expect("--pairs");
    let size: usize = arg("--size", "200").parse().expect("--size");
    let secs: u64 = arg("--secs", "20").parse().expect("--secs");
    let warmup: u64 = arg("--warmup", "3").parse().expect("--warmup");
    let rate: f64 = arg("--rate", "0").parse().expect("--rate");
    let poll_ms: u64 = arg("--poll-ms", "20").parse().expect("--poll-ms");
    // > 0: o destinatario usa WaitMessages (entrega imediata) em vez de perguntar.
    let wait_secs: u64 = arg("--wait-secs", "0").parse().expect("--wait-secs");
    assert!(size >= 16, "--size >= 16 (os primeiros bytes levam o id e a hora de envio)");

    let start = Instant::now();
    let counting = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let c = Arc::new(Counters::default());
    let samples = Arc::new(Samples { rtt_us: Mutex::new(Vec::new()), e2e_us: Mutex::new(Vec::new()) });

    let mut tasks = Vec::new();
    for p in 0..pairs {
        let sender = SigningKeyPair::generate();
        let receiver = SigningKeyPair::generate();
        let to = user_id_from_signing_public(&receiver.verifying_key.to_bytes());
        let (mut ws_s, mut n_s) = login(&url, &sender).await.expect("ligacao do remetente");
        let (mut ws_r, mut n_r) = login(&url, &receiver).await.expect("ligacao do destinatario");

        // Remetente.
        {
            let (c, samples, counting, stop) = (c.clone(), samples.clone(), counting.clone(), stop.clone());
            tasks.push(tokio::spawn(async move {
                let mut seq: u64 = (p as u64) << 40;
                let gap = if rate > 0.0 { Some(Duration::from_secs_f64(1.0 / rate)) } else { None };
                let mut next = tokio::time::Instant::now();
                let mut body = vec![0u8; size];
                for (i, b) in body.iter_mut().enumerate() {
                    *b = (i * 31 + p) as u8;
                }
                while !stop.load(Relaxed) {
                    if let Some(g) = gap {
                        next += g;
                        tokio::time::sleep_until(next).await;
                    }
                    seq += 1;
                    let t = start.elapsed().as_nanos() as u64;
                    body[..8].copy_from_slice(&seq.to_be_bytes());
                    body[8..16].copy_from_slice(&t.to_be_bytes());
                    let m = ClientMessage::SendMessage { to: to.clone(), sealed_from: vec![], ciphertext: body.clone() };
                    let t0 = Instant::now();
                    match req(&mut ws_s, &mut n_s, &m).await {
                        Ok(ServerMessage::Ack) => {
                            if counting.load(Relaxed) {
                                c.sent.fetch_add(1, Relaxed);
                                c.bytes_out.fetch_add(size as u64, Relaxed);
                                samples.rtt_us.lock().unwrap().push(t0.elapsed().as_micros() as u64);
                            }
                        }
                        Ok(_) => {
                            c.send_errors.fetch_add(1, Relaxed);
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                        Err(_) => {
                            c.send_errors.fetch_add(1, Relaxed);
                            return;
                        }
                    }
                }
            }));
        }
        // Destinatario.
        {
            let (c, samples, counting, stop) = (c.clone(), samples.clone(), counting.clone(), stop.clone());
            let me = user_id_from_signing_public(&receiver.verifying_key.to_bytes());
            tasks.push(tokio::spawn(async move {
                let mut seen: HashSet<u64> = HashSet::new();
                // Depois de parar os envios, ainda esvazia a fila durante 2 s.
                let mut drain_until: Option<Instant> = None;
                loop {
                    if stop.load(Relaxed) {
                        let d = *drain_until.get_or_insert_with(|| Instant::now() + Duration::from_secs(2));
                        if Instant::now() > d {
                            return;
                        }
                    }
                    let pedido = if wait_secs > 0 {
                        ClientMessage::WaitMessages { user_id: me.clone(), timeout_secs: wait_secs.min(2) }
                    } else {
                        ClientMessage::FetchMessages { user_id: me.clone() }
                    };
                    let r = match req(&mut ws_r, &mut n_r, &pedido).await {
                        Ok(r) => r,
                        Err(_) => return,
                    };
                    c.fetches.fetch_add(1, Relaxed);
                    let ServerMessage::Messages { messages } = r else { continue };
                    if messages.is_empty() {
                        if wait_secs == 0 {
                            tokio::time::sleep(Duration::from_millis(poll_ms)).await;
                        }
                        continue;
                    }
                    let now = start.elapsed().as_nanos() as u64;
                    let mut ids = Vec::with_capacity(messages.len());
                    for m in &messages {
                        ids.push(m.id);
                        let seq = u64::from_be_bytes(m.ciphertext[..8].try_into().unwrap());
                        let t = u64::from_be_bytes(m.ciphertext[8..16].try_into().unwrap());
                        if !seen.insert(seq) {
                            c.duplicates.fetch_add(1, Relaxed);
                            continue;
                        }
                        if counting.load(Relaxed) {
                            c.delivered.fetch_add(1, Relaxed);
                            samples.e2e_us.lock().unwrap().push(now.saturating_sub(t) / 1000);
                        }
                    }
                    if req(&mut ws_r, &mut n_r, &ClientMessage::AckMessages { user_id: me.clone(), ids }).await.is_err() {
                        return;
                    }
                }
            }));
        }
    }

    let modo = if wait_secs > 0 { "entrega imediata (WaitMessages)".to_string() } else { format!("pedidos a cada {poll_ms} ms quando vazio") };
    println!("relay {url} | pares {pairs} | tamanho {size} B | ritmo {} | {modo} | {secs} s (+{warmup} s de aquecimento)",
        if rate > 0.0 { format!("{rate} msg/s por remetente") } else { "o mais depressa possivel".into() });
    tokio::time::sleep(Duration::from_secs(warmup)).await;
    counting.store(true, Relaxed);
    let t0 = Instant::now();
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let window = t0.elapsed().as_secs_f64();
    counting.store(false, Relaxed);
    stop.store(true, Relaxed);
    for t in tasks {
        let _ = t.await;
    }

    let sent = c.sent.load(Relaxed);
    let delivered = c.delivered.load(Relaxed);
    let mut rtt = samples.rtt_us.lock().unwrap().clone();
    let mut e2e = samples.e2e_us.lock().unwrap().clone();
    println!("envios confirmados: {sent} ({:.0}/s, {:.1} MB/s) | erros: {}", sent as f64 / window, c.bytes_out.load(Relaxed) as f64 / window / 1e6, c.send_errors.load(Relaxed));
    println!("entregues: {delivered} ({:.0}/s) | repetidas: {} | pedidos de mensagens: {}", delivered as f64 / window, c.duplicates.load(Relaxed), c.fetches.load(Relaxed));
    println!("resposta a um envio (ms): p50 {:.2} | p95 {:.2} | p99 {:.2} | max {:.2}", pct(&mut rtt, 0.5), pct(&mut rtt, 0.95), pct(&mut rtt, 0.99), pct(&mut rtt, 1.0));
    println!("ponta a ponta (ms):       p50 {:.2} | p95 {:.2} | p99 {:.2} | max {:.2}", pct(&mut e2e, 0.5), pct(&mut e2e, 0.95), pct(&mut e2e, 0.99), pct(&mut e2e, 1.0));
}
