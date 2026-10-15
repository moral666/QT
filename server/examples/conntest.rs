//! MEDICAO DE LIGACOES EM SIMULTANEO no relay.
//!
//!   cargo run --release -p qt_server --example conntest -- \
//!       --relay ws://127.0.0.1:9443 --conns 10000 --poll-ms 3000 --secs 90
//!
//! Abre `--conns` ligacoes ao relay pelo mesmo caminho que as apps (WebSocket ->
//! Noise XX -> autenticacao com assinatura) e mantem-nas abertas durante `--secs`
//! segundos. Cada ligacao pede as suas mensagens a cada `--poll-ms` ms, como a app
//! faz enquanto esta aberta (3000 ms); com `--poll-ms 0` as ligacoes ficam paradas.
//! Imprime, a cada segundo, quantas estao abertas, quantas falharam e os pedidos por
//! segundo. A memoria e o CPU medem-se de fora, no processo do relay
//! (ver docs/performance.md).
//!
//! Simplificacoes (de proposito, para medir so o relay): todas as ligacoes usam a
//! mesma identidade, e o cliente corre sem TLS contra o relay local. Com muitas
//! ligacoes e preciso subir o limite de ficheiros abertos (`ulimit -n`) no cliente
//! e no relay, e `RELAY_MAX_CONNECTIONS` no relay.

use futures_util::{SinkExt, StreamExt};
use qt_ffi::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

static OPEN: AtomicUsize = AtomicUsize::new(0);
static FAILED: AtomicUsize = AtomicUsize::new(0);
static REQUESTS: AtomicUsize = AtomicUsize::new(0);

fn arg(name: &str, default: &str) -> String {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned()).unwrap_or_else(|| default.to_string())
}

fn s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Atraso aleatorio em [0, max) ms, para os pedidos nao chegarem todos ao mesmo tempo.
fn jitter_ms(max: u64) -> Result<u64, String> {
    if max == 0 {
        return Ok(0);
    }
    let r = qt_transport::generate_static_keypair().map_err(s)?.private;
    let mut b = [0u8; 8];
    b.copy_from_slice(&r[..8]);
    Ok(u64::from_le_bytes(b) % max)
}

async fn next_binary<S>(ws: &mut S) -> Result<Vec<u8>, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match ws.next().await {
            Some(Ok(Message::Binary(b))) => return Ok(b),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            other => return Err(format!("resposta inesperada: {other:?}")),
        }
    }
}

async fn one_connection(qt: Arc<QtClient>, url: String, poll_ms: u64, secs: u64) -> Result<(), String> {
    // Handshake Noise XX
    let noise = qt.relay_noise_session().map_err(s)?;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.map_err(s)?;
    while !noise.is_finished() {
        ws.send(Message::Binary(noise.write_step().map_err(s)?)).await.map_err(s)?;
        if !noise.is_finished() {
            noise.read_step(next_binary(&mut ws).await?).map_err(s)?;
        }
    }
    // Autenticacao (assinatura sobre o hash do handshake)
    let auth = qt.relay_auth_message(noise.handshake_hash().map_err(s)?).map_err(s)?;
    ws.send(Message::Binary(noise.encrypt(auth).map_err(s)?)).await.map_err(s)?;
    match relay_parse_response(noise.decrypt(next_binary(&mut ws).await?).map_err(s)?).map_err(s)? {
        RelayResponse::Ack => {}
        _ => return Err("autenticacao recusada".into()),
    }

    OPEN.fetch_add(1, Ordering::SeqCst);
    let end = tokio::time::Instant::now() + Duration::from_secs(secs);
    let result = async {
        tokio::time::sleep(Duration::from_millis(jitter_ms(poll_ms)?)).await;
        while tokio::time::Instant::now() < end {
            if poll_ms == 0 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            let req = qt.relay_request_fetch().map_err(s)?;
            ws.send(Message::Binary(noise.encrypt(req).map_err(s)?)).await.map_err(s)?;
            noise.decrypt(next_binary(&mut ws).await?).map_err(s)?;
            REQUESTS.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(poll_ms)).await;
        }
        Ok::<(), String>(())
    }
    .await;
    OPEN.fetch_sub(1, Ordering::SeqCst);
    let _ = ws.close(None).await;
    result
}

#[tokio::main]
async fn main() {
    let url = arg("--relay", "ws://127.0.0.1:9443");
    let conns: usize = arg("--conns", "1000").parse().expect("--conns");
    let poll_ms: u64 = arg("--poll-ms", "3000").parse().expect("--poll-ms");
    let secs: u64 = arg("--secs", "60").parse().expect("--secs");
    assert!((1..=50_000).contains(&conns), "--conns entre 1 e 50000");

    let dir = std::env::temp_dir().join(format!("qt-conntest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("pasta temporaria");
    let key = qt_transport::generate_static_keypair().expect("rng").private;
    let qt = QtClient::open(dir.join("u.db").to_string_lossy().to_string(), key).expect("storage");
    qt.create_identity().expect("identidade");
    println!("Relay: {url}   ligacoes: {conns}   pedido a cada: {poll_ms} ms   duracao: {secs} s\n");

    let mut tasks = Vec::with_capacity(conns);
    for i in 0..conns {
        let (qt, url) = (qt.clone(), url.clone());
        tasks.push(tokio::spawn(async move {
            if let Err(e) = one_connection(qt, url, poll_ms, secs).await {
                if FAILED.fetch_add(1, Ordering::SeqCst) < 3 {
                    eprintln!("falha: {e}");
                }
            }
        }));
        // Abre as ligacoes aos poucos (200 a cada 100 ms), como utilizadores a chegar.
        if i % 200 == 199 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    let (mut last, mut max_open) = (0usize, 0usize);
    for _ in 0..secs + 60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let (open, req) = (OPEN.load(Ordering::SeqCst), REQUESTS.load(Ordering::Relaxed));
        max_open = max_open.max(open);
        println!("abertas {open:>6}   falhas {:>5}   pedidos/s {:>6}", FAILED.load(Ordering::SeqCst), req - last);
        last = req;
        if tasks.iter().all(|t| t.is_finished()) {
            break;
        }
    }
    println!(
        "\nResumo: maximo de ligacoes abertas em simultaneo {max_open} de {conns}; falhas {}; pedidos respondidos {}",
        FAILED.load(Ordering::SeqCst),
        REQUESTS.load(Ordering::Relaxed)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
