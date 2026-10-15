//! TESTE PROLONGADO do relay e do nucleo, pelo mesmo caminho que as apps.
//!
//!   redis-server --port 6391 --appendonly yes --daemonize yes
//!   cargo build --release -p qt_server --bin relay_server --example soak
//!   ./target/release/examples/soak --pairs 10 --minutes 30 --kill-every-s 180 --down-s 10
//!
//! Varios pares de utilizadores a conversar durante N minutos,
//! pelo mesmo caminho que as apps (QtClient + Noise + relay real + Redis), com o
//! relay morto (kill -9) e reiniciado de tempos a tempos (queda de rede).
//! No fim verifica que cada mensagem chegou exatamente uma vez.
use futures_util::{SinkExt, StreamExt};
use qt_ffi::*;
use qt_ffi::history::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn arg(name: &str, default: &str) -> String {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned()).unwrap_or_else(|| default.to_string())
}
fn s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}
fn rnd(max: u64) -> u64 {
    let k = qt_transport::generate_static_keypair().unwrap().private;
    u64::from_le_bytes(k[..8].try_into().unwrap()) % max.max(1)
}
fn rss_kb(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|t| t.lines().find(|l| l.starts_with("VmRSS")).map(|l| l.split_whitespace().nth(1).unwrap().parse().unwrap()))
        .unwrap_or(0)
}

struct Conn {
    ws: Ws,
    noise: Arc<NoiseSession>,
}
impl Conn {
    async fn open(qt: &Arc<QtClient>, url: &str) -> Result<Conn, String> {
        let noise = qt.relay_noise_session().map_err(s)?;
        let (mut ws, _) = tokio::time::timeout(Duration::from_secs(5), tokio_tungstenite::connect_async(url))
            .await
            .map_err(|_| "timeout ao ligar".to_string())?
            .map_err(s)?;
        while !noise.is_finished() {
            ws.send(Message::Binary(noise.write_step().map_err(s)?)).await.map_err(s)?;
            if !noise.is_finished() {
                match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
                    Ok(Some(Ok(Message::Binary(b)))) => noise.read_step(b).map_err(s)?,
                    _ => return Err("handshake".into()),
                }
            }
        }
        let mut c = Conn { ws, noise };
        let auth = qt.relay_auth_message(c.noise.handshake_hash().map_err(s)?).map_err(s)?;
        match c.req(auth).await? {
            RelayResponse::Ack => Ok(c),
            _ => Err("auth".into()),
        }
    }
    async fn req(&mut self, bytes: Vec<u8>) -> Result<RelayResponse, String> {
        self.ws.send(Message::Binary(self.noise.encrypt(bytes).map_err(s)?)).await.map_err(s)?;
        loop {
            match tokio::time::timeout(Duration::from_secs(10), self.ws.next()).await {
                Err(_) => return Err("timeout".into()),
                Ok(None) => return Err("fechada".into()),
                Ok(Some(Err(e))) => return Err(s(e)),
                Ok(Some(Ok(Message::Binary(b)))) => return relay_parse_response(self.noise.decrypt(b).map_err(s)?).map_err(s),
                Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
                Ok(Some(Ok(o))) => return Err(format!("frame {o:?}")),
            }
        }
    }
}

#[derive(Default)]
struct Stats {
    sent: AtomicU64,
    received: AtomicU64,
    reconnects: AtomicU64,
    conn_errors: AtomicU64,
    replay: AtomicU64,
    other_receive_errors: AtomicU64,
    error_kinds: Mutex<HashMap<String, u64>>,
    latency_ms: Mutex<Vec<u64>>,
}

struct User {
    qt: Arc<QtClient>,
    pid: String,
    peer_pid: String,
    peer_relay: String,
}

fn spawn_relay(bin: &str, dir: &std::path::Path, port: u16, redis: &str) -> std::process::Child {
    std::process::Command::new(bin)
        .env("RELAY_BIND_ADDR", format!("127.0.0.1:{port}"))
        .env("REDIS_URL", redis)
        .env("NOISE_KEY_PATH", dir.join("relay_key.bin"))
        .env("RELAY_MAX_CONNECTIONS", "1000")
        .stdout(std::fs::File::create(dir.join("relay.log")).unwrap())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("relay")
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let bin = arg("--relay-bin", "target/release/relay_server");
    let redis = arg("--redis", "redis://127.0.0.1:6391");
    let port: u16 = arg("--port", "9460").parse().unwrap();
    let pairs: usize = arg("--pairs", "10").parse().unwrap();
    let minutes: u64 = arg("--minutes", "30").parse().unwrap();
    let kill_every: u64 = arg("--kill-every-s", "180").parse().unwrap();
    let down_s: u64 = arg("--down-s", "8").parse().unwrap();
    let poll_ms: u64 = arg("--poll-ms", "500").parse().unwrap();
    let url = format!("ws://127.0.0.1:{port}");
    let dir = std::env::temp_dir().join(format!("qt-soak-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let relay = Arc::new(Mutex::new(spawn_relay(&bin, &dir, port, &redis)));
    tokio::time::sleep(Duration::from_millis(800)).await;

    // Utilizadores: pares (2k, 2k+1); cada um publica o bundle.
    let mut qts = Vec::new();
    for i in 0..pairs * 2 {
        let qt = QtClient::open(dir.join(format!("u{i}.db")).to_string_lossy().to_string(), qt_transport::generate_static_keypair().unwrap().private).unwrap();
        let me = qt.create_identity().unwrap();
        let mut c = Conn::open(&qt, &url).await.expect("ligar");
        assert!(matches!(c.req(qt.relay_request_register_bundle(qt.publishable_bundle().unwrap()).unwrap()).await.unwrap(), RelayResponse::Ack));
        qts.push((qt, me.public_id));
    }
    let mut users = Vec::new();
    for i in 0..pairs * 2 {
        let j = i ^ 1;
        users.push(Arc::new(User {
            qt: qts[i].0.clone(),
            pid: qts[i].1.clone(),
            peer_pid: qts[j].1.clone(),
            peer_relay: relay_id_of_public_id(qts[j].1.clone()).unwrap(),
        }));
    }

    let stats = Arc::new(Stats::default());
    let sending = Arc::new(AtomicBool::new(true));
    let running = Arc::new(AtomicBool::new(true));
    let sent_log: Arc<Mutex<HashMap<String, u64>>> = Arc::new(Mutex::new(HashMap::new()));
    let t0 = Instant::now();

    // Cada utilizador: o mesmo ciclo que a app (flush da outbox, buscar, decifrar, confirmar, recibos).
    let mut tasks = Vec::new();
    for (idx, u) in users.iter().enumerate() {
        let (u, url, st, sending, running, log) = (u.clone(), url.clone(), stats.clone(), sending.clone(), running.clone(), sent_log.clone());
        tasks.push(tokio::spawn(async move {
            let mut conn: Option<Conn> = None;
            let mut n = 0u64;
            let mut next_send = Instant::now() + Duration::from_millis(rnd(2000));
            let mut burst_at = Instant::now() + Duration::from_secs(60 + rnd(60));
            let peer_cid = u.peer_pid[4..68].to_string();
            while running.load(Ordering::Relaxed) {
                if conn.is_none() {
                    match Conn::open(&u.qt, &url).await {
                        Ok(c) => {
                            conn = Some(c);
                            st.reconnects.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => {
                            st.conn_errors.fetch_add(1, Ordering::Relaxed);
                            tokio::time::sleep(Duration::from_millis(500 + rnd(1500))).await;
                            continue;
                        }
                    }
                }
                let c = conn.as_mut().unwrap();
                let mut broken = false;
                // Novas mensagens (como o utilizador a escrever), mais rajadas de 30.
                if sending.load(Ordering::Relaxed) && Instant::now() >= next_send {
                    let count = if Instant::now() >= burst_at { burst_at = Instant::now() + Duration::from_secs(120 + rnd(120)); 30 } else { 1 };
                    if !u.qt.has_session(peer_cid.clone()).unwrap() {
                        match c.req(u.qt.relay_request_fetch_bundle(u.peer_relay.clone()).unwrap()).await {
                            Ok(RelayResponse::Bundle { bytes }) => {
                                let _ = u.qt.start_session(u.peer_pid.clone(), peer_bundle_from_relay_bytes(bytes).unwrap());
                            }
                            Ok(_) => {}
                            Err(_) => broken = true,
                        }
                    }
                    if !broken && u.qt.has_session(peer_cid.clone()).unwrap() {
                        for _ in 0..count {
                            let text = format!("{idx}|{n}|{}", t0.elapsed().as_millis());
                            u.qt.send_text(peer_cid.clone(), text).unwrap();
                            log.lock().unwrap().insert(format!("{idx}|{n}"), 0);
                            n += 1;
                            st.sent.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    next_send = Instant::now() + Duration::from_millis(500 + rnd(4000));
                }
                // Flush da outbox (mensagens e recibos), pela ordem.
                if !broken {
                    for item in u.qt.outbox().unwrap() {
                        match c.req(u.qt.relay_request_send(u.peer_relay.clone(), item.blob.clone()).unwrap()).await {
                            Ok(RelayResponse::Ack) => u.qt.outbox_sent(item.outbox_id).unwrap(),
                            Ok(_) => {}
                            Err(_) => { broken = true; break; }
                        }
                    }
                }
                // Buscar, decifrar, guardar e so depois confirmar.
                if !broken {
                    match c.req(u.qt.relay_request_fetch().unwrap()).await {
                        Ok(RelayResponse::Messages { messages }) if !messages.is_empty() => {
                            let mut acks = Vec::new();
                            for m in messages {
                                match u.qt.receive_sealed(m.ciphertext) {
                                    Ok(ReceiveResult::NewMessage { message }) => {
                                        st.received.fetch_add(1, Ordering::Relaxed);
                                        let parts: Vec<&str> = message.text.split('|').collect();
                                        if parts.len() == 3 {
                                            let ms: u64 = parts[2].parse().unwrap_or(0);
                                            st.latency_ms.lock().unwrap().push((t0.elapsed().as_millis() as u64).saturating_sub(ms));
                                        }
                                        acks.push(m.id);
                                    }
                                    Ok(_) => acks.push(m.id),
                                    Err(QtError::Replay) => { st.replay.fetch_add(1, Ordering::Relaxed); acks.push(m.id); }
                                    Err(QtError::Storage) => {}
                                    Err(e) => { st.other_receive_errors.fetch_add(1, Ordering::Relaxed); *st.error_kinds.lock().unwrap().entry(format!("{e:?}")).or_default() += 1; acks.push(m.id); }
                                }
                            }
                            if c.req(u.qt.relay_request_ack(acks).unwrap()).await.is_err() { broken = true; }
                            let _ = u.qt.prepare_delivery_receipts();
                            if rnd(4) == 0 { let _ = u.qt.mark_conversation_read(peer_cid.clone(), true); }
                        }
                        Ok(_) => {}
                        Err(_) => broken = true,
                    }
                }
                if broken {
                    conn = None;
                }
                tokio::time::sleep(Duration::from_millis(poll_ms)).await;
            }
        }));
    }

    // Quedas do relay + medicoes a cada minuto.
    let me = std::process::id();
    let mut kills = 0;
    let end_send = Instant::now() + Duration::from_secs(minutes * 60);
    let mut next_kill = Instant::now() + Duration::from_secs(kill_every);
    let mut next_report = Instant::now() + Duration::from_secs(60);
    println!("min | enviadas | recebidas | religacoes | erros ligacao | replays | outros | RSS teste MB | RSS relay MB | quedas");
    while Instant::now() < end_send {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if kill_every > 0 && Instant::now() >= next_kill {
            {
                let mut r = relay.lock().unwrap();
                let _ = r.kill();
                let _ = r.wait();
            }
            kills += 1;
            tokio::time::sleep(Duration::from_secs(down_s)).await;
            *relay.lock().unwrap() = spawn_relay(&bin, &dir, port, &redis);
            next_kill = Instant::now() + Duration::from_secs(kill_every);
        }
        if Instant::now() >= next_report {
            let rpid = relay.lock().unwrap().id();
            println!(
                "{:>3} | {:>8} | {:>9} | {:>10} | {:>13} | {:>7} | {:>6} | {:>12.1} | {:>12.1} | {kills}",
                t0.elapsed().as_secs() / 60,
                stats.sent.load(Ordering::Relaxed), stats.received.load(Ordering::Relaxed),
                stats.reconnects.load(Ordering::Relaxed), stats.conn_errors.load(Ordering::Relaxed),
                stats.replay.load(Ordering::Relaxed), stats.other_receive_errors.load(Ordering::Relaxed),
                rss_kb(me) as f64 / 1024.0, rss_kb(rpid) as f64 / 1024.0,
            );
            next_report = Instant::now() + Duration::from_secs(60);
        }
    }
    // Para de enviar e espera que tudo drene (sem quedas).
    sending.store(false, Ordering::Relaxed);
    let drain_start = Instant::now();
    while drain_start.elapsed() < Duration::from_secs(120) {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if stats.received.load(Ordering::Relaxed) >= stats.sent.load(Ordering::Relaxed)
            && users.iter().all(|u| u.qt.outbox().unwrap().is_empty())
        {
            tokio::time::sleep(Duration::from_secs(3)).await;
            break;
        }
    }
    running.store(false, Ordering::Relaxed);
    for t in tasks {
        let _ = t.await;
    }

    // Verificacao exata: cada mensagem enviada aparece uma vez no historico do destinatario.
    let mut missing = 0u64;
    let mut dup = 0u64;
    let mut not_delivered = 0u64;
    for (idx, u) in users.iter().enumerate() {
        let peer = &users[idx ^ 1];
        let my_cid = u.pid[4..68].to_string();
        let mut seen: HashMap<String, u32> = HashMap::new();
        let mut before = None;
        loop {
            let page = peer.qt.messages_page(my_cid.clone(), before, 500).unwrap();
            if page.is_empty() { break; }
            for m in &page {
                if !m.outgoing {
                    let k: Vec<&str> = m.text.splitn(3, '|').collect();
                    *seen.entry(format!("{}|{}", k[0], k[1])).or_default() += 1;
                }
            }
            before = Some(page.last().unwrap().seq);
        }
        let log = sent_log.lock().unwrap();
        for key in log.keys().filter(|k| k.starts_with(&format!("{idx}|"))) {
            match seen.get(key) {
                None => missing += 1,
                Some(1) => {}
                Some(_) => dup += 1,
            }
        }
        // Estado das enviadas: todas devem chegar a Entregue/Visto.
        let peer_cid = u.peer_pid[4..68].to_string();
        let mut before = None;
        loop {
            let page = u.qt.messages_page(peer_cid.clone(), before, 500).unwrap();
            if page.is_empty() { break; }
            not_delivered += page.iter().filter(|m| m.outgoing && !matches!(m.status, MessageStatus::Delivered | MessageStatus::Read)).count() as u64;
            before = Some(page.last().unwrap().seq);
        }
    }
    let mut lat = stats.latency_ms.lock().unwrap().clone();
    lat.sort();
    let p = |q: f64| lat.get(((lat.len() as f64 - 1.0) * q) as usize).copied().unwrap_or(0);
    println!("\nRESUMO: {} min, {} pares, {} quedas do relay", minutes, pairs, kills);
    println!("enviadas {}  recebidas {}  em falta {}  duplicadas {}  enviadas sem 'Entregue' {}", stats.sent.load(Ordering::Relaxed), stats.received.load(Ordering::Relaxed), missing, dup, not_delivered);
    println!("erros na rececao por tipo: {:?}", stats.error_kinds.lock().unwrap());
    println!("latencia envio->rececao: p50 {} ms  p95 {} ms  p99 {} ms  max {} ms (inclui o tempo com o relay em baixo)", p(0.5), p(0.95), p(0.99), lat.last().copied().unwrap_or(0));
    let _ = relay.lock().unwrap().kill();
    let _ = std::fs::remove_dir_all(&dir);
}
