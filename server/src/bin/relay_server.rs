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
    enforce_bind_policy_for("RELAY_BIND_ADDR", bind_addr);
}

fn enforce_bind_policy_for(var: &str, bind_addr: &str) {
    let addr: std::net::SocketAddr = bind_addr
        .parse()
        .unwrap_or_else(|_| fatal(&format!("{var} invalido (esperado IP:porta, ex.: 127.0.0.1:9443)")));
    if !addr.ip().is_loopback() && !env_flag("QT_ALLOW_PUBLIC_BIND") {
        fatal(&format!("{var} nao e loopback: o relay nao tem TLS proprio e tem de ficar atras do proxy (ou QT_ALLOW_PUBLIC_BIND=1 em testes)"));
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

/// Conta do operador (denuncias e banimentos): a chave publica Ed25519, em hex, no ficheiro
/// `RELAY_OPERATOR_FILE` (por omissao `/etc/qt/operator.pub`, fora do git). Sem ficheiro, os
/// pedidos de operador ficam desligados; as denuncias das apps continuam a ser guardadas.
fn load_operator() -> Option<String> {
    let path = std::env::var("RELAY_OPERATOR_FILE").unwrap_or_else(|_| "/etc/qt/operator.pub".to_string());
    let text = std::fs::read_to_string(&path).ok()?;
    let id = text.trim().to_ascii_lowercase();
    if qt_server::protocol::is_valid_user_id(&id) {
        println!("Operador: denuncias e banimentos ligados ({}...)", &id[..8]);
        Some(id)
    } else {
        eprintln!("Operador: {path} invalido (esperava 64 caracteres hex); pedidos de operador desligados");
        None
    }
}

/// `qt-relay chaves-fichas [meses]`: escreve no ecra o ficheiro do site com as chaves publicas das
/// fichas do video (`site/public/estado.json`), do mes atual em diante (24 meses por omissao).
/// So le a semente (`FICHAS_SEED_PATH`, ou ao lado da chave Noise); nunca a cria.
fn print_published_keys(args: &[String]) -> ! {
    let months: u32 = args.get(2).and_then(|v| v.parse().ok()).filter(|m| (1..=120).contains(m)).unwrap_or(24);
    let noise_key_path = std::env::var("NOISE_KEY_PATH").unwrap_or_else(|_| "relay_noise_key.bin".to_string());
    let path = std::env::var("FICHAS_SEED_PATH").map(std::path::PathBuf::from).unwrap_or_else(|_| {
        std::path::Path::new(&noise_key_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.join("fichas_seed.bin"))
            .unwrap_or_else(|| std::path::PathBuf::from("fichas_seed.bin"))
    });
    let seed = qt_server::fichas::load_seed(&path).unwrap_or_else(|_| fatal("nao foi possivel ler a semente das fichas (FICHAS_SEED_PATH)"));
    print!("{}", qt_server::fichas::published_keys_json(&seed, qt_core::fichas::current_epoch(), months));
    std::process::exit(0);
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("chaves-fichas") {
        print_published_keys(&args);
    }
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
    // Notificacoes do iPhone (Apple, sem Firebase). Sem QT_APNS_CREDENTIALS ficam desligadas.
    match qt_server::apns::init_from_env() {
        Ok(true) => println!("Notificacoes do iPhone (APNs): ativas"),
        Ok(false) => println!("Notificacoes do iPhone (APNs): desligadas (falta QT_APNS_CREDENTIALS)"),
        Err(e) => fatal(&format!("notificacoes do iPhone: {e}")),
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
    // definido, o JSON de estado que o HAProxy publica em /estado.json.
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
                last = Some(qt_server::latency::LastReading { avg_ms: ms, median_ms: summary.median_ms, at_unix: unix_now() });
            }
            if let Some(ms) = ida.avg_ms {
                last_ida = Some(qt_server::latency::LastReading { avg_ms: ms, median_ms: ida.median_ms, at_unix: unix_now() });
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
    // Limites contra abusos (limits.rs): ligacoes por autenticar e por conta.
    let defaults = qt_server::ServerConfig::default();
    let cfg = Arc::new(qt_server::ServerConfig {
        max_unauthenticated: std::env::var("RELAY_MAX_UNAUTHENTICATED")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(defaults.max_unauthenticated),
        max_connections_per_user: std::env::var("RELAY_MAX_CONNECTIONS_PER_USER")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(defaults.max_connections_per_user),
        operator: load_operator(),
        fichas: load_fichas(&noise_key_path),
        federacao: arrancar_federacao(&store, &static_keys.private).await,
        fichas_per_month: std::env::var("RELAY_FICHAS_POR_MES")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(defaults.fichas_per_month),
        ..defaults
    });
    if cfg.fichas.is_some() {
        println!(
            "Fichas do video: ligadas, {} por conta e por mes ({} min de video pelo relay)",
            cfg.fichas_per_month,
            cfg.fichas_per_month as u64 * qt_core::fichas::BLOCK_SECS / 60
        );
    }
    println!(
        "Limites: {max_conns} ligacoes, {} por autenticar, {} por conta",
        cfg.max_unauthenticated, cfg.max_connections_per_user
    );

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
        let cfg = cfg.clone();

        tokio::spawn(async move {
            let stats = &qt_server::stats::STATS;
            stats.active.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // O erro e descartado de proposito: nao e registado para nao
            // criar um log de metadados por ligacao.
            let _ = qt_server::handle_connection_with(tcp_stream, store, static_private, &cfg).await;
            stats.active.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            drop(permit);
        });
    }
}

/// Federacao de relays (`federation.rs`): `RELAY_FED_FILE` (por omissao
/// `/etc/qt/federacao.json`, fora do git). Sem ficheiro, o relay fica sozinho (como antes).
/// Os pares ligam-se a `RELAY_FED_BIND` (por omissao 127.0.0.1:9446, atras do HAProxy `/fed`).
async fn arrancar_federacao(
    store: &Arc<Store>,
    chave_privada: &[u8],
) -> Option<Arc<qt_server::federation::Federacao>> {
    let path = std::env::var("RELAY_FED_FILE").unwrap_or_else(|_| "/etc/qt/federacao.json".to_string());
    let cfg = match qt_server::federation::FedConfig::ler(std::path::Path::new(&path)) {
        Ok(Some(c)) => c,
        Ok(None) => {
            println!("Federacao: desligada (sem {path})");
            return None;
        }
        Err(e) => fatal(&format!("federacao: {e}")),
    };
    let bind = std::env::var("RELAY_FED_BIND").unwrap_or_else(|_| "127.0.0.1:9446".to_string());
    enforce_bind_policy_for("RELAY_FED_BIND", &bind);
    let ouvinte = TcpListener::bind(&bind)
        .await
        .unwrap_or_else(|_| fatal("federacao: nao foi possivel abrir RELAY_FED_BIND (porta ocupada?)"));
    let pares: Vec<String> = cfg.pares.iter().map(|p| p.nome.clone()).collect();
    println!("Federacao: este relay e \"{}\", pares: {} (a ouvir em {bind})", cfg.nome, pares.join(", "));
    Some(qt_server::federation::Federacao::arrancar(cfg, store.clone(), chave_privada.to_vec(), ouvinte))
}

/// Semente das chaves das fichas do video (0006): `FICHAS_SEED_PATH`, por omissao
/// `fichas_seed.bin` ao lado da chave Noise (em producao /var/lib/qt). Criada no primeiro
/// arranque (0600); o relay das chamadas so a le. `RELAY_FICHAS=0` desliga a emissao.
fn load_fichas(noise_key_path: &str) -> Option<Arc<qt_server::fichas::IssuerKeys>> {
    if matches!(std::env::var("RELAY_FICHAS").as_deref(), Ok("0") | Ok("false") | Ok("no")) {
        println!("Fichas do video: desligadas (RELAY_FICHAS=0)");
        return None;
    }
    let path = std::env::var("FICHAS_SEED_PATH").map(std::path::PathBuf::from).unwrap_or_else(|_| {
        std::path::Path::new(noise_key_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.join("fichas_seed.bin"))
            .unwrap_or_else(|| std::path::PathBuf::from("fichas_seed.bin"))
    });
    match qt_server::fichas::load_or_create_seed(&path) {
        Ok(seed) => Some(Arc::new(qt_server::fichas::IssuerKeys::new(*seed))),
        Err(_) => {
            eprintln!("AVISO: nao foi possivel ler/criar {}: fichas do video desligadas", path.display());
            None
        }
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
