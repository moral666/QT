//! Liga o handshake/transporte Noise (noise_session.rs) a um WebSocket real,
//! usando `tokio-tungstenite`. Cada mensagem Noise viaja como um frame
//! binario do WebSocket.
//!
//! NOTA DE SEGURANCA: esta demo usa `ws://` (sem TLS) sobre localhost, para
//! poder ser testada neste ambiente sem certificados. Em producao, usar
//! sempre `wss://` (WebSocket sobre TLS) como primeira camada de defesa de
//! rede, com o Noise por cima como segunda camada (autenticacao mutua que
//! nao depende da CA/PKI do TLS).

use crate::noise_session::{NoiseError, NoiseHandshake, NoiseTransport};
use futures_util::{SinkExt, StreamExt};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("erro Noise: {0}")]
    Noise(#[from] NoiseError),
    #[error("erro WebSocket: {0}")]
    WebSocket(#[from] Box<tokio_tungstenite::tungstenite::Error>),
    #[error("ligacao fechada inesperadamente durante o handshake")]
    ConnectionClosedDuringHandshake,
    #[error("tipo de frame WebSocket inesperado (esperava binario)")]
    UnexpectedFrameType,
}

pub type ClientWsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
pub type ServerWsStream = WebSocketStream<TcpStream>;

/// Lado CLIENTE: liga a um servidor WebSocket e executa o handshake Noise
/// como iniciador. Devolve o stream (para continuar a enviar/receber frames
/// brutos, se necessario) e o `NoiseTransport` já pronto para cifrar/decifrar.
pub async fn client_connect(
    url: &str,
    local_static_private: &[u8],
) -> Result<(ClientWsStream, NoiseTransport), TransportError> {
    let (mut ws_stream, _response) = tokio_tungstenite::connect_async(url).await.map_err(Box::new)?;

    let mut handshake = NoiseHandshake::new_initiator(local_static_private)?;

    // Padrao Noise_XX: iniciador escreve primeiro.
    while !handshake.is_finished() {
        let out = handshake.write_step()?;
        ws_stream.send(Message::Binary(out)).await.map_err(Box::new)?;

        if !handshake.is_finished() {
            let msg = ws_stream
                .next()
                .await
                .ok_or(TransportError::ConnectionClosedDuringHandshake)?
            .map_err(Box::new)?;
            let bytes = expect_binary(msg)?;
            handshake.read_step(&bytes)?;
        }
    }

    let transport = handshake.into_transport()?;
    Ok((ws_stream, transport))
}

/// Lado SERVIDOR: aceita uma ligacao TCP ja promovida a WebSocket (feito
/// pelo chamador via `tokio_tungstenite::accept_async`) e executa o
/// handshake Noise como responder.
pub async fn server_accept(
    tcp_stream: TcpStream,
    local_static_private: &[u8],
) -> Result<(ServerWsStream, NoiseTransport), TransportError> {
    let mut ws_stream = tokio_tungstenite::accept_async(tcp_stream).await.map_err(Box::new)?;

    let mut handshake = NoiseHandshake::new_responder(local_static_private)?;

    // Padrao Noise_XX: responder le primeiro, depois escreve, etc.
    while !handshake.is_finished() {
        let msg = ws_stream
            .next()
            .await
            .ok_or(TransportError::ConnectionClosedDuringHandshake)?
            .map_err(Box::new)?;
        let bytes = expect_binary(msg)?;
        handshake.read_step(&bytes)?;

        if !handshake.is_finished() {
            let out = handshake.write_step()?;
            ws_stream.send(Message::Binary(out)).await.map_err(Box::new)?;
        }
    }

    let transport = handshake.into_transport()?;
    Ok((ws_stream, transport))
}

fn expect_binary(msg: Message) -> Result<Vec<u8>, TransportError> {
    match msg {
        Message::Binary(b) => Ok(b),
        _ => Err(TransportError::UnexpectedFrameType),
    }
}

/// Envia bytes de aplicacao (ja cifrados pelo Double Ratchet em `core/`)
/// atraves do canal Noise, como um unico frame WebSocket binario.
pub async fn send_encrypted<S>(
    ws_stream: &mut WebSocketStream<S>,
    transport: &mut NoiseTransport,
    plaintext_for_noise_layer: &[u8],
) -> Result<(), TransportError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let ciphertext = transport.encrypt(plaintext_for_noise_layer)?;
    ws_stream.send(Message::Binary(ciphertext)).await.map_err(Box::new)?;
    Ok(())
}

/// Recebe e decifra o proximo frame do canal Noise.
pub async fn receive_encrypted<S>(
    ws_stream: &mut WebSocketStream<S>,
    transport: &mut NoiseTransport,
) -> Result<Vec<u8>, TransportError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let msg = ws_stream
        .next()
        .await
        .ok_or(TransportError::ConnectionClosedDuringHandshake)?
            .map_err(Box::new)?;
    let bytes = expect_binary(msg)?;
    Ok(transport.decrypt(&bytes)?)
}
