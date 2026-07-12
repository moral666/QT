//! Trata uma unica ligacao de cliente: faz o handshake Noise (via
//! `transport/`) e depois entra num loop a processar `ClientMessage`s,
//! respondendo com `ServerMessage`s - tudo dentro do canal Noise ja
//! estabelecido.

use crate::protocol::{
    deserialize_client_message, serialize_server_message, ClientMessage, DeliveredMessage,
    ServerMessage,
};
use crate::store::Store;
use qt_transport::ws_transport;
use std::sync::Arc;
use thiserror::Error;
use tokio::net::TcpStream;

#[derive(Debug, Error)]
pub enum ConnectionError {
    #[error("erro de transporte: {0}")]
    Transport(#[from] ws_transport::TransportError),
    #[error("mensagem do cliente malformada: {0}")]
    Deserialize(#[from] serde_json::Error),
}

/// Ponto de entrada chamado para cada nova ligacao TCP aceite pelo servidor.
/// Faz o handshake Noise como responder e depois processa mensagens ate a
/// ligacao fechar.
pub async fn handle_connection(
    tcp_stream: TcpStream,
    store: Arc<Store>,
    server_static_private: Vec<u8>,
) -> Result<(), ConnectionError> {
    let (mut ws_stream, mut noise_transport) =
        ws_transport::server_accept(tcp_stream, &server_static_private).await?;

    loop {
        let payload = match ws_transport::receive_encrypted(&mut ws_stream, &mut noise_transport).await
        {
            Ok(p) => p,
            Err(_) => break, // ligacao fechada pelo cliente - encerra normalmente
        };

        let client_msg = match deserialize_client_message(&payload) {
            Ok(m) => m,
            Err(e) => {
                // Mensagem malformada: responde com erro, mas nao derruba a
                // ligacao inteira por uma mensagem invalida isolada.
                let err_msg = ServerMessage::Error { reason: format!("mensagem invalida: {e}") };
                let bytes = serialize_server_message(&err_msg);
                ws_transport::send_encrypted(&mut ws_stream, &mut noise_transport, &bytes).await?;
                continue;
            }
        };

        let response = handle_client_message(&store, client_msg).await;
        let response_bytes = serialize_server_message(&response);
        ws_transport::send_encrypted(&mut ws_stream, &mut noise_transport, &response_bytes).await?;
    }

    Ok(())
}

async fn handle_client_message(store: &Store, msg: ClientMessage) -> ServerMessage {
    match msg {
        ClientMessage::RegisterPreKeyBundle { user_id, bundle_bytes } => {
            match store.register_bundle(user_id, bundle_bytes).await {
                Ok(()) => ServerMessage::Ack,
                Err(e) => ServerMessage::Error { reason: e.to_string() },
            }
        }
        ClientMessage::FetchPreKeyBundle { user_id } => match store.get_bundle(&user_id).await {
            Ok(Some(bundle_bytes)) => ServerMessage::PreKeyBundle { bundle_bytes },
            Ok(None) => ServerMessage::PreKeyBundleNotFound,
            Err(e) => ServerMessage::Error { reason: e.to_string() },
        },
        ClientMessage::SendMessage { to, sealed_from, ciphertext } => {
            match store.enqueue_message(to, sealed_from, ciphertext).await {
                Ok(()) => ServerMessage::Ack,
                Err(reason) => ServerMessage::Error { reason: reason.to_string() },
            }
        }
        ClientMessage::FetchMessages { user_id } => match store.drain_messages(&user_id).await {
            Ok(raw) => {
                let messages = raw
                    .into_iter()
                    .map(|(sealed_from, ciphertext)| DeliveredMessage { sealed_from, ciphertext })
                    .collect();
                ServerMessage::Messages { messages }
            }
            Err(e) => ServerMessage::Error { reason: e.to_string() },
        },
    }
}
