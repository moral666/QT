//! MEDICAO DE DESEMPENHO do relay, pelo MESMO caminho da app Android:
//! QtClient (PQXDH + Double Ratchet + Sealed Sender) -> Noise -> WebSocket (wss://) -> relay -> Redis.
//!
//!   cargo run --release -p qt_server --example loadtest -- \
//!       --relay wss://<o-teu-relay> --pin <64 hex> --pairs 5 --msgs 20
//!
//! Cria `--pairs` pares de utilizadores temporarios (A_i -> B_i), cada um com a sua ligacao
//! persistente ao relay (como a app). Cada A_i envia `--msgs` mensagens de `--size` bytes a B_i.
//! Os B_i vao buscar as mensagens a cada `--poll-ms` ms (a app usa ~3000 ms).
//!
//! Mede: ligacao (TLS + Noise + autenticacao), tempo de cifrar/decifrar no cliente, tempo de
//! resposta do relay a um envio (RTT), latencia de entrega ponta a ponta e mensagens perdidas.
//! Usa so dados temporarios (apagados no fim). Carga por omissao: pequena (5 pares x 20 msgs).

use futures_util::{SinkExt, StreamExt};
use qt_ffi::*;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn arg(name: &str, default: &str) -> String {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned()).unwrap_or_else(|| default.to_string())
}

/// Ligacao persistente ao relay, autenticada (igual ao que a app faz).
struct Conn {
    ws: Ws,
    noise: Arc<NoiseSession>,
}

impl Conn {
    async fn open(qt: &Arc<QtClient>, url: &str, pin: &Option<String>) -> Result<Conn, String> {
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
        let mut c = Conn { ws, noise };
        let auth = qt.relay_auth_message(c.noise.handshake_hash().map_err(s)?).map_err(s)?;
        match c.req(auth).await? {
            RelayResponse::Ack => Ok(c),
            _ => Err("autenticacao recusada".into()),
        }
    }

    async fn req(&mut self, bytes: Vec<u8>) -> Result<RelayResponse, String> {
        self.ws.send(Message::Binary(self.noise.encrypt(bytes).map_err(s)?)).await.map_err(s)?;
        loop {
            match tokio::time::timeout(Duration::from_secs(15), self.ws.next()).await {
                Err(_) => return Err("timeout".into()),
                Ok(None) => return Err("ligacao fechada".into()),
                Ok(Some(Err(e))) => return Err(s(e)),
                Ok(Some(Ok(Message::Binary(b)))) => return relay_parse_response(self.noise.decrypt(b).map_err(s)?).map_err(s),
                Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
                Ok(Some(Ok(other))) => return Err(format!("frame inesperado: {other:?}")),
            }
        }
    }
}

#[derive(Default)]
struct Stats {
    connect_ms: Vec<f64>,
    encrypt_ms: Vec<f64>,
    decrypt_ms: Vec<f64>,
    send_rtt_ms: Vec<f64>,
    fetch_rtt_ms: Vec<f64>,
    delivery_ms: Vec<f64>,
    errors: Vec<String>,
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let i = ((p / 100.0) * (v.len() as f64 - 1.0)).round() as usize;
    v[i]
}

fn line(name: &str, v: &mut [f64]) {
    if v.is_empty() {
        println!("  {name:<34} (sem dados)");
        return;
    }
    let (p50, p95, p99, max) = (pct(v, 50.0), pct(v, 95.0), pct(v, 99.0), pct(v, 100.0));
    println!("  {name:<34} p50 {p50:>8.2} ms   p95 {p95:>8.2} ms   p99 {p99:>8.2} ms   max {max:>8.2} ms   (n={})", v.len());
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

#[tokio::main]
async fn main() {
    let url = arg("--relay", "ws://127.0.0.1:9443");
    let pin = Some(arg("--pin", "")).filter(|p| !p.is_empty());
    let pairs: usize = arg("--pairs", "5").parse().expect("--pairs");
    let msgs: usize = arg("--msgs", "20").parse().expect("--msgs");
    let size: usize = arg("--size", "200").parse().expect("--size");
    let poll_ms: u64 = arg("--poll-ms", "100").parse().expect("--poll-ms");
    let send_gap_ms: u64 = arg("--gap-ms", "50").parse().expect("--gap-ms");
    assert!((1..=50).contains(&pairs), "--pairs entre 1 e 50");
    assert!((1..=500).contains(&msgs), "--msgs entre 1 e 500 (a fila do relay aceita 1000 por utilizador)");
    assert!((16..=8000).contains(&size), "--size entre 16 e 8000 bytes");

    let dir = std::env::temp_dir().join(format!("qt-loadtest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("pasta temporaria");
    println!("Relay: {url}   pares: {pairs}   mensagens/par: {msgs}   tamanho: {size} B   poll: {poll_ms} ms\n");

    let stats = Arc::new(Mutex::new(Stats::default()));
    let t0 = Instant::now();

    // ---------- 1. Utilizadores: identidade + ligacao + publicar bundle ----------
    let mut clients = Vec::new();
    for i in 0..pairs * 2 {
        let key = qt_transport::generate_static_keypair().expect("rng").private;
        let qt = QtClient::open(dir.join(format!("u{i}.db")).to_string_lossy().to_string(), key).expect("storage");
        qt.create_identity().expect("identidade");
        let t = Instant::now();
        let mut conn = match Conn::open(&qt, &url, &pin).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("ERRO a ligar ao relay: {e}");
                let _ = std::fs::remove_dir_all(&dir);
                std::process::exit(1);
            }
        };
        stats.lock().await.connect_ms.push(ms(t.elapsed()));
        match conn.req(qt.relay_request_register_bundle(qt.publishable_bundle().expect("bundle")).expect("req")).await {
            Ok(RelayResponse::Ack) => {}
            other => panic!("registo do bundle falhou: {:?}", other.map(|_| ())),
        }
        let pid = qt.export_public_identity().expect("id").public_id;
        clients.push((qt, Arc::new(Mutex::new(conn)), pid));
    }
    println!("[1/3] {} utilizadores ligados e registados", clients.len());

    // ---------- 2. Sessoes PQXDH A_i -> B_i ----------
    for p in 0..pairs {
        let (qa, ca, _) = &clients[2 * p];
        let (_, _, pid_b) = &clients[2 * p + 1];
        let relay_b = relay_id_of_public_id(pid_b.clone()).unwrap();
        let r = ca.lock().await.req(qa.relay_request_fetch_bundle(relay_b).unwrap()).await;
        match r {
            Ok(RelayResponse::Bundle { bytes }) => {
                qa.start_session(pid_b.clone(), peer_bundle_from_relay_bytes(bytes).unwrap()).expect("sessao");
            }
            _ => panic!("bundle do destinatario nao encontrado"),
        }
    }
    println!("[2/3] {pairs} sessoes PQXDH estabelecidas");

    // ---------- 3. Envio + rececao concorrentes ----------
    let total = pairs * msgs;
    let received = Arc::new(Mutex::new(0usize));
    let deadline = Instant::now() + Duration::from_secs(30 + (msgs as u64 * send_gap_ms) / 1000 * 2);
    let mut tasks = Vec::new();
    for p in 0..pairs {
        // Remetente
        let (qa, ca, _) = clients[2 * p].clone();
        let pid_b = clients[2 * p + 1].2.clone();
        let st = stats.clone();
        tasks.push(tokio::spawn(async move {
            let relay_b = relay_id_of_public_id(pid_b.clone()).unwrap();
            let cid = pid_b[4..68].to_string();
            for n in 0..msgs {
                let mut body = format!("{}|{}|", t0.elapsed().as_nanos(), n).into_bytes();
                body.resize(size, b'.');
                let t = Instant::now();
                let blob = qa.encrypt_sealed(cid.clone(), body).expect("cifrar");
                let enc = ms(t.elapsed());
                let req = qa.relay_request_send(relay_b.clone(), blob).unwrap();
                let t = Instant::now();
                let r = ca.lock().await.req(req).await;
                let rtt = ms(t.elapsed());
                let mut g = st.lock().await;
                g.encrypt_ms.push(enc);
                match r {
                    Ok(RelayResponse::Ack) => g.send_rtt_ms.push(rtt),
                    Ok(_) => g.errors.push("envio: resposta inesperada".into()),
                    Err(e) => g.errors.push(format!("envio: {e}")),
                }
                drop(g);
                tokio::time::sleep(Duration::from_millis(send_gap_ms)).await;
            }
        }));
        // Destinatario
        let (qb, cb, _) = clients[2 * p + 1].clone();
        let (st, rec) = (stats.clone(), received.clone());
        tasks.push(tokio::spawn(async move {
            let mut got = 0usize;
            while got < msgs && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(poll_ms)).await;
                let t = Instant::now();
                let r = cb.lock().await.req(qb.relay_request_fetch().unwrap()).await;
                let rtt = ms(t.elapsed());
                let msgs_in = match r {
                    Ok(RelayResponse::Messages { messages }) => messages,
                    Ok(_) => continue,
                    Err(e) => {
                        st.lock().await.errors.push(format!("rececao: {e}"));
                        continue;
                    }
                };
                st.lock().await.fetch_rtt_ms.push(rtt);
                let mut ids = Vec::new();
                for m in msgs_in {
                    let t = Instant::now();
                    match qb.decrypt_sealed(m.ciphertext) {
                        Ok(d) => {
                            let dec = ms(t.elapsed());
                            let now = t0.elapsed().as_nanos();
                            let txt = String::from_utf8_lossy(&d.plaintext);
                            let sent: u128 = txt.split('|').next().and_then(|x| x.parse().ok()).unwrap_or(now);
                            let mut g = st.lock().await;
                            g.decrypt_ms.push(dec);
                            g.delivery_ms.push((now - sent) as f64 / 1e6);
                            got += 1;
                        }
                        Err(e) => st.lock().await.errors.push(format!("decifrar: {e}")),
                    }
                    ids.push(m.id);
                }
                if !ids.is_empty() {
                    let _ = cb.lock().await.req(qb.relay_request_ack(ids).unwrap()).await;
                }
            }
            *rec.lock().await += got;
        }));
    }
    let t_phase = Instant::now();
    for t in tasks {
        let _ = t.await;
    }
    let elapsed = t_phase.elapsed();
    println!("[3/3] envio e rececao concluidos em {:.1} s\n", elapsed.as_secs_f64());

    // ---------- Relatorio ----------
    let mut g = stats.lock().await;
    let got = *received.lock().await;
    println!("=== RESULTADOS ===");
    line("Ligacao (TLS+Noise+autenticacao)", &mut g.connect_ms);
    line("Cifrar mensagem (cliente)", &mut g.encrypt_ms);
    line("Decifrar mensagem (cliente)", &mut g.decrypt_ms);
    line("Envio -> confirmacao do relay", &mut g.send_rtt_ms);
    line("Pedido de rececao (fetch)", &mut g.fetch_rtt_ms);
    line("Entrega ponta a ponta", &mut g.delivery_ms);
    println!("  (a entrega inclui ate {poll_ms} ms de espera do poll; na app o poll e de ~3000 ms)");
    println!("\n  Mensagens entregues: {got}/{total}   perdidas: {}", total - got);
    println!("  Debito: {:.1} mensagens/s", got as f64 / elapsed.as_secs_f64());
    println!("  Erros: {}", g.errors.len());
    for e in g.errors.iter().take(10) {
        println!("    - {e}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
