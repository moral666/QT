//! Binario standalone do servidor/relay. Uso:
//!   cargo run --bin relay_server
//!
//! Gera um par de chaves estaticas Noise NOVO a cada arranque (nao
//! persistido) - isto e adequado so para desenvolvimento. Em producao, a
//! chave estatica do servidor deve ser gerada uma vez e persistida (para
//! que os clientes possam reconhecer o mesmo servidor entre reinicios via
//! "pinning" - ver transport/src/noise_session.rs, remote_static_public_key).

use secure_messenger_server::Store;
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let bind_addr = std::env::var("RELAY_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9443".to_string());

    let static_keys = secure_messenger_transport::generate_static_keypair()
        .expect("falha ao gerar par de chaves estaticas Noise do servidor");

    println!("Servidor/relay a arrancar em {bind_addr}");
    println!(
        "Chave publica estatica Noise (partilha isto com clientes para pinning): {}",
        hex_encode(&static_keys.public)
    );
    println!("AVISO: chave gerada de novo a cada arranque - so adequado para desenvolvimento.");

    let store = Arc::new(Store::new());
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
                secure_messenger_server::handle_connection(tcp_stream, store, static_private).await
            {
                eprintln!("ligacao de {peer_addr} terminou com erro: {e}");
            }
        });
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
