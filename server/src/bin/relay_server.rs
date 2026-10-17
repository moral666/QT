//! Binario standalone do servidor/relay. Uso:
//!   REDIS_URL=redis://127.0.0.1:6379 cargo run --bin relay_server
//!
//! TLS: o relay escuta em TCP simples e, POR OMISSAO, so em 127.0.0.1. Os
//! clientes ligam por `wss://` a um proxy inverso que termina o TLS e
//! encaminha para aqui (ver docs/deploy-wss.md). So uses RELAY_BIND_ADDR=0.0.0.0:...
//! em redes de teste fechadas.
//!
//! Requer um Redis a correr (ver README.md) - a fila de mensagens e o
//! diretorio de pre-keys sao persistidos la, nao em memoria.
//!
//! A chave estatica Noise do servidor e agora PERSISTIDA num ficheiro
//! local (`NOISE_KEY_PATH`, por omissao `relay_noise_key.bin`) - gerada
//! uma unica vez no primeiro arranque, e reutilizada em arranques
//! seguintes, para que os clientes possam confiar na mesma identidade de
//! transporte do servidor entre reinicios ("pinning" - ver
//! transport/src/noise_session.rs, remote_static_public_key).
//!
//! AVISO DE SEGURANCA: o ficheiro guarda a chave privada em texto simples
//! no disco (apenas com permissoes 0600 no Unix). Isto e aceitavel para
//! um servidor operado por uma organizacao com acesso fisico controlado
//! ao disco, mas nao e o nivel de protecao usado para chaves de
//! utilizador final (essas usam SQLCipher - ver storage/).

use qt_server::Store;
use qt_transport::{generate_static_keypair, static_keypair_from_private_bytes, NoiseStaticKeyPair};
use std::sync::Arc;
use tokio::net::TcpListener;

/// Erro fatal de arranque: mensagem curta e saida != 0 (sem panic/backtrace).
fn fatal(msg: &str) -> ! {
    eprintln!("erro fatal: {msg}");
    std::process::exit(1);
}

/// Escreve o ficheiro com permissoes 0600 DESDE A CRIACAO (sem janela em que
/// a chave privada fica legivel por outros utilizadores).
fn write_private_file(path: &str, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(data)
}

fn load_or_generate_static_keys(path: &str) -> NoiseStaticKeyPair {
    match std::fs::read(path) {
        Ok(private_bytes) => {
            println!("Chave estatica Noise carregada de {path}");
            static_keypair_from_private_bytes(&private_bytes)
                .unwrap_or_else(|_| fatal("ficheiro de chave Noise existente esta corrompido"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("Nenhuma chave estatica em {path} - a gerar uma nova (primeiro arranque)...");
            let keys = generate_static_keypair().unwrap_or_else(|_| fatal("falha ao gerar chaves Noise"));
            write_private_file(path, &keys.private)
                .unwrap_or_else(|_| fatal("falha ao guardar a chave Noise (permissoes/disco)"));
            keys
        }
        Err(_) => fatal("nao foi possivel ler o ficheiro da chave Noise"),
    }
}

/// Host de um URL `esquema://[user[:pass]@]host[:porta][/...]`, sem
/// parentesis retos no caso IPv6. `None` para URLs sem autoridade.
fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next()?
    } else {
        authority.split(':').next()?
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn is_loopback_host(host: &str) -> bool {
    host == "localhost" || host.parse::<std::net::IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

fn env_flag(name: &str) -> bool {
    matches!(std::env::var(name).as_deref(), Ok("1") | Ok("true") | Ok("yes"))
}

/// Isolamento de rede: por omissao o relay SO aceita um Redis local
/// (loopback ou socket Unix). Um Redis remoto (ex.: ElastiCache numa VPC
/// privada) exige opt-in explicito QT_ALLOW_REMOTE_REDIS=1 E TLS (`rediss://`).
fn enforce_redis_url_policy(redis_url: &str) {
    let lower = redis_url.to_ascii_lowercase();
    if lower.starts_with("unix://") || lower.starts_with("redis+unix://") {
        return;
    }
    let Some(host) = url_host(redis_url) else { fatal("REDIS_URL invalido") };
    if is_loopback_host(&host) {
        return;
    }
    if !env_flag("QT_ALLOW_REMOTE_REDIS") {
        fatal("REDIS_URL aponta para um host nao-loopback; o Redis deve escutar so em 127.0.0.1 (ou define QT_ALLOW_REMOTE_REDIS=1 com rediss://)");
    }
    if !lower.starts_with("rediss://") {
        fatal("Redis remoto exige TLS (rediss://)");
    }
}

/// O relay fala TCP simples (sem TLS): so pode escutar em loopback, atras
/// do proxy inverso que termina o TLS. Abrir noutra interface exige
/// QT_ALLOW_PUBLIC_BIND=1 (apenas redes de teste fechadas).
fn enforce_bind_policy(bind_addr: &str) {
    let addr: std::net::SocketAddr = bind_addr
        .parse()
        .unwrap_or_else(|_| fatal("RELAY_BIND_ADDR invalido (esperado IP:porta, ex.: 127.0.0.1:9443)"));
    if !addr.ip().is_loopback() && !env_flag("QT_ALLOW_PUBLIC_BIND") {
        fatal("RELAY_BIND_ADDR nao e loopback: o relay nao tem TLS proprio e tem de ficar atras do proxy (ou QT_ALLOW_PUBLIC_BIND=1 em testes)");
    }
}

/// Remove credenciais de um URL do Redis antes de o mostrar (redis://user:pass@host).
fn redact_url(url: &str) -> String {
    match (url.find("://"), url.rfind('@')) {
        (Some(i), Some(at)) if at > i => format!("{}://***@{}", &url[..i], &url[at + 1..]),
        _ => url.to_string(),
    }
}

/// Escreve o JSON de estado (tempo medio de entrega); uma falha so fica no registo.
fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn write_status(
    path: &str,
    summary: &qt_server::latency::Summary,
    last: Option<qt_server::latency::LastReading>,
    ida: &qt_server::latency::Summary,
    last_ida: Option<qt_server::latency::LastReading>,
) {
    let json = qt_server::latency::to_json(summary, unix_now(), last, ida, last_ida);
    if qt_server::latency::write_status_file(path, &json).is_err() {
        eprintln!("estado: nao foi possivel escrever RELAY_STATUS_FILE");
    }
}

#[tokio::main]
async fn main() {
    let bind_addr = std::env::var("RELAY_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:9443".to_string());
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let noise_key_path = std::env::var("NOISE_KEY_PATH").unwrap_or_else(|_| "relay_noise_key.bin".to_string());

    enforce_bind_policy(&bind_addr);
    enforce_redis_url_policy(&redis_url);

    let static_keys = load_or_generate_static_keys(&noise_key_path);

    println!("Servidor/relay a arrancar em {bind_addr}");
    println!("A usar Redis em: {}", redact_url(&redis_url));
    println!(
        "Chave publica estatica Noise (partilha isto com clientes para pinning): {}",
        hex_encode(&static_keys.public)
    );

    let mut store = Store::new(&redis_url).unwrap_or_else(|_| fatal("URL do Redis invalido"));
    configure_blobs(&mut store, &noise_key_path);
    // Notificacoes com a app fechada (FCM). Sem QT_FCM_CREDENTIALS ficam desligadas.
    match qt_server::push::init_from_env() {
        Ok(true) => println!("Notificacoes (FCM): ativas"),
        Ok(false) => println!("Notificacoes (FCM): desligadas (falta QT_FCM_CREDENTIALS)"),
        Err(e) => fatal(&format!("notificacoes: {e}")),
    }
    let store = Arc::new(store);
    if store.blobs_enabled() {
        // Limpeza dos anexos: ao arrancar e de 10 em 10 minutos (expirados e orfaos).
        let s = store.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(600));
            loop {
                tick.tick().await;
                match s.sweep_blobs().await {
                    Ok((removed, used)) if removed > 0 => {
                        println!("anexos: {removed} expirados apagados, {} MB reservados", used / 1_048_576)
                    }
                    Ok(_) => {}
                    Err(_) => eprintln!("anexos: falha na limpeza (Redis/disco)"),
                }
            }
        });
    }
    let listener = TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|_| fatal("nao foi possivel abrir o socket TCP (porta ocupada/permissoes?)"));

    // Estatisticas anonimas (so contagens), uma linha por minuto. No mesmo ciclo,
    // a media do tempo de entrega (latency.rs) e, se RELAY_STATUS_FILE estiver
    // definido, o JSON de estado que o Caddy publica em /estado.json.
    let status_file = std::env::var("RELAY_STATUS_FILE").ok().filter(|p| !p.is_empty());
    // Ultima media conhecida: recuperada do ficheiro de estado, para nao se perder ao reiniciar.
    let previous = status_file.as_deref().and_then(|p| std::fs::read_to_string(p).ok());
    let mut last = previous.as_deref().and_then(qt_server::latency::parse_last);
    let mut last_ida = previous.as_deref().and_then(qt_server::latency::parse_last_ida);
    if let Some(path) = &status_file {
        let empty = qt_server::latency::Summary::default();
        write_status(path, &empty, last, &empty, last_ida);
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        tick.tick().await;
        loop {
            tick.tick().await;
            let (summary, ida) = qt_server::latency::roll_minute();
            if let Some(mut line) = qt_server::stats::take_line(&qt_server::stats::STATS) {
                if let (Some(avg), Some(med), Some(p95)) = (ida.avg_ms, ida.median_ms, ida.p95_ms) {
                    line.push_str(&format!(" ida_media_ms={avg} ida_mediana_ms={med} ida_p95_ms={p95} ({} amostras)", ida.messages));
                }
                if let (Some(avg), Some(med), Some(p95)) = (summary.avg_ms, summary.median_ms, summary.p95_ms) {
                    line.push_str(&format!(" entrega_media_ms={avg} entrega_mediana_ms={med} entrega_p95_ms={p95} ({} mensagens)", summary.messages));
                }
                println!("{line} (ultimos {} min)", summary.minutes);
            }
            if let Some(ms) = summary.avg_ms {
                last = Some(qt_server::latency::LastReading { avg_ms: ms, at_unix: unix_now() });
            }
            if let Some(ms) = ida.avg_ms {
                last_ida = Some(qt_server::latency::LastReading { avg_ms: ms, at_unix: unix_now() });
            }
            if let Some(path) = &status_file {
                write_status(path, &summary, last, &ida, last_ida);
            }
        }
    });

    // Limite de ligacoes simultaneas: protege a memoria e os descritores de
    // ficheiro (um cliente mal comportado nao derruba o relay para todos).
    let max_conns: usize = std::env::var("RELAY_MAX_CONNECTIONS").ok().and_then(|v| v.parse().ok()).unwrap_or(4096);
    let slots = Arc::new(tokio::sync::Semaphore::new(max_conns));

    // LOGS: so informacao operacional. NUNCA enderecos IP/identificadores de
    // utilizadores, conteudo, chaves ou tokens.
    loop {
        let (tcp_stream, _peer) = match listener.accept().await {
            Ok(x) => x,
            Err(_) => {
                eprintln!("falha ao aceitar ligacao");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };

        let Ok(permit) = slots.clone().try_acquire_owned() else {
            // Cheio: fecha logo (o cliente volta a tentar com backoff).
            qt_server::stats::inc(&qt_server::stats::STATS.rejected_busy);
            drop(tcp_stream);
            continue;
        };
        qt_server::stats::inc(&qt_server::stats::STATS.connections);

        let store = store.clone();
        let static_private = static_keys.private.clone();

        tokio::spawn(async move {
            let stats = &qt_server::stats::STATS;
            stats.active.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // O erro e descartado de proposito: nao e registado para nao
            // criar um log de metadados por ligacao.
            let _ = qt_server::handle_connection(tcp_stream, store, static_private).await;
            stats.active.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            drop(permit);
        });
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

/// Anexos (imagens/GIFs cifrados) no disco, no maximo 24 h. Pasta por omissao:
/// `anexos/` ao lado da chave Noise (em producao /var/lib/qt/anexos). Se a pasta
/// nao puder ser criada, o relay arranca na mesma, sem anexos.
fn configure_blobs(store: &mut Store, noise_key_path: &str) {
    if matches!(std::env::var("RELAY_BLOBS").as_deref(), Ok("0") | Ok("false") | Ok("no")) {
        println!("Anexos desativados (RELAY_BLOBS=0)");
        return;
    }
    let dir = std::env::var("RELAY_BLOB_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| {
        std::path::Path::new(noise_key_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.join("anexos"))
            .unwrap_or_else(|| std::path::PathBuf::from("anexos"))
    });
    let mut cfg = qt_server::BlobConfig::new(&dir);
    cfg.max_total_bytes = env_u64("RELAY_BLOB_MAX_TOTAL_MB", 2048) * 1_048_576;
    cfg.max_user_bytes_per_day = env_u64("RELAY_BLOB_USER_DAILY_MB", 1024) * 1_048_576;
    cfg.min_free_bytes = env_u64("RELAY_BLOB_MIN_FREE_MB", 1024) * 1_048_576;
    let (total_mb, user_mb) = (cfg.max_total_bytes / 1_048_576, cfg.max_user_bytes_per_day / 1_048_576);
    match store.enable_blobs(cfg) {
        Ok(()) => println!(
            "Anexos em {} (24 h; ate {total_mb} MB no total, {user_mb} MB por utilizador por dia, deixando {} MB livres no disco)",
            dir.display(),
            env_u64("RELAY_BLOB_MIN_FREE_MB", 1024)
        ),
        Err(_) => eprintln!("AVISO: nao foi possivel criar {}: anexos desativados", dir.display()),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extrai_host_e_deteta_loopback() {
        assert_eq!(url_host("redis://127.0.0.1:6379").as_deref(), Some("127.0.0.1"));
        assert_eq!(url_host("redis://:segredo@127.0.0.1:6379/0").as_deref(), Some("127.0.0.1"));
        assert_eq!(url_host("redis://u:p@[::1]:6379").as_deref(), Some("::1"));
        assert_eq!(url_host("rediss://cache.xyz.amazonaws.com:6379").as_deref(), Some("cache.xyz.amazonaws.com"));
        // A password com '@' nao pode enganar o parser.
        assert_eq!(url_host("redis://u:p@ss@10.0.0.5:6379").as_deref(), Some("10.0.0.5"));
        assert!(is_loopback_host("127.0.0.1") && is_loopback_host("::1") && is_loopback_host("localhost"));
        for h in ["0.0.0.0", "10.0.0.5", "127.0.0.1.evil.com", "localhost.evil.org"] {
            assert!(!is_loopback_host(h), "{h}");
        }
    }
}
