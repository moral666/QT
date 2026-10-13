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

/// Remove credenciais de um URL do Redis antes de o mostrar (redis://user:pass@host).
fn redact_url(url: &str) -> String {
    match (url.find("://"), url.rfind('@')) {
        (Some(i), Some(at)) if at > i => format!("{}://***@{}", &url[..i], &url[at + 1..]),
        _ => url.to_string(),
    }
}

#[tokio::main]
async fn main() {
    let bind_addr = std::env::var("RELAY_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:9443".to_string());
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let noise_key_path = std::env::var("NOISE_KEY_PATH").unwrap_or_else(|_| "relay_noise_key.bin".to_string());

    let static_keys = load_or_generate_static_keys(&noise_key_path);

    println!("Servidor/relay a arrancar em {bind_addr}");
    println!("A usar Redis em: {}", redact_url(&redis_url));
    println!(
        "Chave publica estatica Noise (partilha isto com clientes para pinning): {}",
        hex_encode(&static_keys.public)
    );

    let store = Arc::new(Store::new(&redis_url).unwrap_or_else(|_| fatal("URL do Redis invalido")));
    let listener = TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|_| fatal("nao foi possivel abrir o socket TCP (porta ocupada/permissoes?)"));

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

        let store = store.clone();
        let static_private = static_keys.private.clone();

        tokio::spawn(async move {
            // O erro e descartado de proposito: nao e registado para nao
            // criar um log de metadados por ligacao.
            let _ = qt_server::handle_connection(tcp_stream, store, static_private).await;
        });
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
