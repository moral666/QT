//! Relay das chamadas de voz (processo a parte do relay das mensagens).
//!
//! Porque separado: o relay das mensagens so fala com localhost (systemd
//! `IPAddressDeny=any`) e tem o Redis das contas; este tem de receber UDP da Internet e
//! nao precisa de nada das contas. Le a chave estatica Noise (a do pin, para abrir os
//! pedidos de entrada nas salas) e, desde o 0006, a semente das fichas (para verificar as
//! fichas do video) e fala com o Redis como `qt-fichas`: so `SET` nas chaves `fichas:*`
//! (registo das fichas gastas), sem acesso as filas nem as contas.
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
//!   CALL_FICHAS               0 = video pelo relay sem fichas, como antes do 0006 (1)
//!   CALL_FICHAS_SEED          semente das fichas, criada pelo qt-relay (/var/lib/qt/fichas_seed.bin)
//!   CALL_REDIS_URL            Redis do registo das gastas, utilizador qt-fichas
//!                             (em /etc/qt-call-relay/fichas.env, gerado pelo setup-ec2.sh)

use qt_server::call_relay::{serve, take_line, CallLimits, FichasSetup};
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
        fichas: env_num("CALL_FICHAS", 1u8) != 0,
        ..defaults
    };
    let fichas = if limits.fichas {
        let seed_path = std::path::PathBuf::from(env_or("CALL_FICHAS_SEED", "/var/lib/qt/fichas_seed.bin"));
        let spent = match std::env::var("CALL_REDIS_URL").ok().filter(|v| !v.trim().is_empty()) {
            Some(url) => {
                if !redis_is_local(&url) {
                    fatal("CALL_REDIS_URL tem de ser o Redis local (127.0.0.1, ::1 ou socket Unix)");
                }
                Some(std::sync::Arc::new(
                    qt_server::fichas::SpentStore::new(&url).unwrap_or_else(|_| fatal("CALL_REDIS_URL invalido")),
                ))
            }
            None => None,
        };
        match (&spent, seed_path.exists()) {
            (None, _) => eprintln!("AVISO: sem CALL_REDIS_URL: nenhuma ficha e aceite (video pelo relay so com CALL_FICHAS=0)"),
            (_, false) => eprintln!("AVISO: ainda sem {} (o qt-relay cria-a ao arrancar): video pelo relay em pausa ate la", seed_path.display()),
            _ => {}
        }
        FichasSetup { seed_path: Some(seed_path), spent }
    } else {
        FichasSetup::default()
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
        "Video: {}, {} bytes/s por lado, travao do mes aos {} GB (depois so 240p), {}",
        if limits.relay_video { "ligado" } else { "desligado (so voz)" },
        limits.video_bytes_per_sec,
        limits.monthly_egress_bytes / 1_000_000_000,
        if limits.fichas { "com fichas (teto da conta)" } else { "sem fichas" }
    );

    let stats = serve(udp, Some(ws), &private, limits, fichas);
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.tick().await;
    loop {
        tick.tick().await;
        if let Some(line) = take_line(&stats) {
            println!("{line} (ultimo minuto)");
        }
    }
}

/// O registo das gastas so pode estar na propria maquina.
fn redis_is_local(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("unix://") || lower.starts_with("redis+unix://") {
        return true;
    }
    let Some(rest) = lower.split_once("://").map(|(_, r)| r) else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::redis_is_local;

    #[test]
    fn registo_das_gastas_so_local() {
        assert!(redis_is_local("redis://qt-fichas:ab@127.0.0.1:6379"));
        assert!(redis_is_local("redis://[::1]:6379") && redis_is_local("unix:///run/redis.sock"));
        assert!(!redis_is_local("redis://qt-fichas:ab@10.0.0.5:6379"));
        assert!(!redis_is_local("redis://u:p@127.0.0.1.evil.com:6379") && !redis_is_local("nada"));
    }
}
