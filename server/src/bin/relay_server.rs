//! Binario standalone do servidor/relay. Uso:
//!   REDIS_URL=redis://127.0.0.1:6379 cargo run --bin relay_server
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

fn load_or_generate_static_keys(path: &str) -> NoiseStaticKeyPair {
    match std::fs::read(path) {
        Ok(private_bytes) => {
            println!("Chave estatica Noise carregada de {path}");
            static_keypair_from_private_bytes(&private_bytes)
                .expect("ficheiro de chave existente esta corrompido")
        }
        Err(_) => {
            println!("Nenhuma chave estatica encontrada em {path} - a gerar uma nova (primeiro arranque)...");
            let keys = generate_static_keypair().expect("falha ao gerar par de chaves estaticas Noise");

            std::fs::write(path, &keys.private).expect("falha ao guardar a chave estatica Noise em disco");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(path).unwrap().permissions();
                perms.set_mode(0o600); // so o dono consegue ler/escrever
                std::fs::set_permissions(path, perms).expect("falha ao restringir permissoes do ficheiro de chave");
            }
            keys
        }
    }
}

#[tokio::main]
async fn main() {
    let bind_addr = std::env::var("RELAY_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9443".to_string());
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let noise_key_path = std::env::var("NOISE_KEY_PATH").unwrap_or_else(|_| "relay_noise_key.bin".to_string());

    let static_keys = load_or_generate_static_keys(&noise_key_path);

    println!("Servidor/relay a arrancar em {bind_addr}");
    println!("A usar Redis em: {redis_url}");
    println!(
        "Chave publica estatica Noise (partilha isto com clientes para pinning): {}",
        hex_encode(&static_keys.public)
    );

    let store = Arc::new(Store::new(&redis_url).expect("falha ao ligar ao Redis"));
    let listener = TcpListener::bind(&bind_addr).await.expect("falha ao ligar o socket TCP");

    loop {
        let (tcp_stream, peer_addr) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                eprintln!("falha ao aceitar ligacao: {e}");
                continue;
            }
        };

        let store = store.clone();
        let static_private = static_keys.private.clone();

        tokio::spawn(async move {
            println!("nova ligacao de {peer_addr}");
            if let Err(e) =
                qt_server::handle_connection(tcp_stream, store, static_private).await
            {
                eprintln!("ligacao de {peer_addr} terminou com erro: {e}");
            }
        });
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
