//! Relay das chamadas de voz (processo a parte do relay das mensagens).
//!
//! Porque separado: o relay das mensagens so fala com localhost (systemd
//! `IPAddressDeny=any`) e tem o Redis; este tem de receber UDP da Internet, mas
//! nao precisa do Redis nem de nada das contas. So le a chave estatica Noise (a do
//! pin, para abrir os pedidos de entrada nas salas).
//!
//! Variaveis (em /etc/qt/relay.env):
//!   NOISE_KEY_PATH   chave estatica do relay (tem de existir; nunca gera uma nova)
//!   CALL_UDP_BIND    UDP publico (por omissao 0.0.0.0:443)
//!   CALL_WS_BIND     reserva por WebSocket, atras do HAProxy (por omissao 127.0.0.1:9445)
//!   CALL_MAX_ROOMS, CALL_ROOMS_PER_IP, CALL_PACKETS_PER_SEC  limites
//!   CALL_VIDEO_BYTES_PER_SEC  video por lado (150000 = ~1,2 Mbit/s)
//!   CALL_MONTHLY_EGRESS_GB    travao do mes: passado isto, video so a 240p (80)
//!   CALL_RELAY_VIDEO          0 = nenhuma sala passa video, so voz (1)
//!   CALL_STATE_FILE           conta do mes (/var/lib/qt-call-relay/mes)

use qt_server::call_relay::{serve, take_line, CallLimits};
use std::time::Duration;

fn fatal(msg: &str) -> ! {
    eprintln!("erro fatal: {msg}");
    std::process::exit(1);
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| default.to_string())
}

fn env_num<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

#[tokio::main]
async fn main() {
    let key_path = env_or("NOISE_KEY_PATH", "relay_noise_key.bin");
    let udp_bind = env_or("CALL_UDP_BIND", "0.0.0.0:443");
    let ws_bind = env_or("CALL_WS_BIND", "127.0.0.1:9445");

    let private = zeroize::Zeroizing::new(
        std::fs::read(&key_path).unwrap_or_else(|_| fatal("nao foi possivel ler a chave Noise do relay (NOISE_KEY_PATH)")),
    );
    let keys = qt_transport::static_keypair_from_private_bytes(&private)
        .unwrap_or_else(|_| fatal("chave Noise do relay invalida"));

    let ws_addr: std::net::SocketAddr = ws_bind.parse().unwrap_or_else(|_| fatal("CALL_WS_BIND invalido"));
    if !ws_addr.ip().is_loopback() {
        fatal("CALL_WS_BIND tem de ser localhost (o TLS e do HAProxy)");
    }

    let defaults = CallLimits::default();
    let limits = CallLimits {
        max_rooms: env_num("CALL_MAX_ROOMS", defaults.max_rooms),
        rooms_per_ip: env_num("CALL_ROOMS_PER_IP", defaults.rooms_per_ip),
        packets_per_sec: env_num("CALL_PACKETS_PER_SEC", defaults.packets_per_sec),
        video_bytes_per_sec: env_num("CALL_VIDEO_BYTES_PER_SEC", defaults.video_bytes_per_sec),
        monthly_egress_bytes: env_num("CALL_MONTHLY_EGRESS_GB", defaults.monthly_egress_bytes / 1_000_000_000).saturating_mul(1_000_000_000),
        relay_video: env_num("CALL_RELAY_VIDEO", 1u8) != 0,
        state_file: Some(std::path::PathBuf::from(env_or("CALL_STATE_FILE", "/var/lib/qt-call-relay/mes"))),
        ..defaults
    };

    let udp = tokio::net::UdpSocket::bind(&udp_bind)
        .await
        .unwrap_or_else(|_| fatal("nao foi possivel abrir o UDP (porta ocupada ou sem permissao para a 443?)"));
    let ws = tokio::net::TcpListener::bind(ws_addr)
        .await
        .unwrap_or_else(|_| fatal("nao foi possivel abrir a porta da reserva por WebSocket"));

    println!("Relay das chamadas: UDP {udp_bind}, reserva WebSocket {ws_bind}");
    println!("Chave publica (igual ao pin do relay das mensagens): {}", hex(&keys.public));
    println!(
        "Limites: {} salas, {} por IP, {} pacotes/s por lado",
        limits.max_rooms, limits.rooms_per_ip, limits.packets_per_sec
    );
    println!(
        "Video: {}, {} bytes/s por lado, travao do mes aos {} GB (depois so 240p)",
        if limits.relay_video { "ligado" } else { "desligado (so voz)" },
        limits.video_bytes_per_sec,
        limits.monthly_egress_bytes / 1_000_000_000
    );

    let stats = serve(udp, Some(ws), &private, limits);
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.tick().await;
    loop {
        tick.tick().await;
        if let Some(line) = take_line(&stats) {
            println!("{line} (ultimo minuto)");
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
