//! Liga o handshake/transporte Noise (noise_session.rs) a um WebSocket real,
//! usando `tokio-tungstenite`. Cada mensagem Noise viaja como um frame
//! binario do WebSocket.
//!
//! SEGURANCA: o cliente so aceita `wss://` (TLS) e, para desenvolvimento,
//! `ws://` apenas em loopback (ver `check_url_policy`). O Noise corre POR
//! CIMA do TLS como segunda camada (autenticacao mutua + pinning da chave
//! do relay, independente da PKI/CA). O relay escuta em TCP simples e o TLS
//! e terminado por um proxy inverso (ver docs/deploy-wss.md).

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
    #[error("a ligacao foi fechada pelo par")]
    ConnectionClosed,
    #[error("timeout")]
    Timeout,
    #[error("URL recusado: usa wss:// (ws:// so e aceite para localhost)")]
    InsecureUrl,
    #[error("chave estatica do servidor nao corresponde a chave esperada (pinning)")]
    ServerKeyMismatch,
}

/// Tamanho maximo de uma mensagem/frame WebSocket aceite (o Noise limita a
/// 65535 bytes por mensagem; uma pequena folga para o enquadramento).
pub const MAX_WS_MESSAGE: usize = 65_535 + 64;

/// Opcoes do cliente.
#[derive(Clone, Debug)]
pub struct ClientOptions {
    /// Timeout para ligar + handshake Noise.
    pub connect_timeout: std::time::Duration,
    /// Se definido, o handshake falha se a chave estatica do servidor for diferente.
    pub expected_server_static: Option<Vec<u8>>,
    /// Permite `ws://` (sem TLS) para destinos que NAO sejam loopback.
    /// Por omissao `false`: fora de localhost so se aceita `wss://`.
    pub allow_insecure_ws: bool,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self { connect_timeout: std::time::Duration::from_secs(10), expected_server_static: None, allow_insecure_ws: false }
    }
}

fn ws_config() -> tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
    tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
        max_message_size: Some(MAX_WS_MESSAGE),
        max_frame_size: Some(MAX_WS_MESSAGE),
        ..Default::default()
    }
}

/// Politica de esquema: `wss://` sempre; `ws://` so para loopback (testes/dev
/// locais) ou com `allow_insecure_ws`. Qualquer outro esquema e recusado.
pub fn check_url_policy(url: &str, allow_insecure_ws: bool) -> Result<(), TransportError> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("wss://") {
        return Ok(());
    }
    if let Some(rest) = lower.strip_prefix("ws://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = if let Some(v6) = authority.strip_prefix('[') {
            v6.split(']').next().unwrap_or("")
        } else {
            authority.split(':').next().unwrap_or("")
        };
        let loopback = host == "localhost"
            || host.parse::<std::net::IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false);
        if loopback || allow_insecure_ws {
            return Ok(());
        }
    }
    Err(TransportError::InsecureUrl)
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
    client_connect_with(url, local_static_private, &ClientOptions::default()).await
}

/// Como `client_connect`, com timeout e (opcionalmente) pinning da chave
/// estatica do servidor.
pub async fn client_connect_with(
    url: &str,
    local_static_private: &[u8],
    opts: &ClientOptions,
) -> Result<(ClientWsStream, NoiseTransport), TransportError> {
    match tokio::time::timeout(opts.connect_timeout, client_connect_inner(url, local_static_private, opts))
        .await
    {
        Ok(r) => r,
        Err(_) => Err(TransportError::Timeout),
    }
}

async fn client_connect_inner(
    url: &str,
    local_static_private: &[u8],
    opts: &ClientOptions,
) -> Result<(ClientWsStream, NoiseTransport), TransportError> {
    check_url_policy(url, opts.allow_insecure_ws)?;
    let (mut ws_stream, _response) =
        tokio_tungstenite::connect_async_with_config(url, Some(ws_config()), false)
            .await
            .map_err(Box::new)?;

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
    if let Some(expected) = &opts.expected_server_static {
        if transport.remote_static_public_key() != Some(expected.as_slice()) {
            let _ = ws_stream.close(None).await;
            return Err(TransportError::ServerKeyMismatch);
        }
    }
    Ok((ws_stream, transport))
}

/// Lado SERVIDOR: aceita uma ligacao TCP ja promovida a WebSocket (feito
/// pelo chamador via `tokio_tungstenite::accept_async`) e executa o
/// handshake Noise como responder.
pub async fn server_accept(
    tcp_stream: TcpStream,
    local_static_private: &[u8],
) -> Result<(ServerWsStream, NoiseTransport), TransportError> {
    server_accept_with(tcp_stream, local_static_private, std::time::Duration::from_secs(10)).await
}

/// Como `server_accept`, mas com timeout para o upgrade WebSocket + handshake
/// Noise (impede ligacoes "penduradas" que nunca completam o handshake).
pub async fn server_accept_with(
    tcp_stream: TcpStream,
    local_static_private: &[u8],
    handshake_timeout: std::time::Duration,
) -> Result<(ServerWsStream, NoiseTransport), TransportError> {
    match tokio::time::timeout(handshake_timeout, server_accept_inner(tcp_stream, local_static_private)).await
    {
        Ok(r) => r,
        Err(_) => Err(TransportError::Timeout),
    }
}

async fn server_accept_inner(
    tcp_stream: TcpStream,
    local_static_private: &[u8],
) -> Result<(ServerWsStream, NoiseTransport), TransportError> {
    let mut ws_stream = tokio_tungstenite::accept_async_with_config(tcp_stream, Some(ws_config()))
        .await
        .map_err(Box::new)?;

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

/// Recebe e decifra o proximo frame do canal Noise. Frames de controlo
/// (Ping/Pong) sao ignorados; um Close devolve `ConnectionClosed`. Qualquer
/// frame que nao decifre (corrompido, duplicado/repetido, fora de ordem -
/// o Noise usa um contador de nonce estrito) devolve erro: o chamador deve
/// ENCERRAR a sessao e re-ligar com um novo handshake.
pub async fn receive_encrypted<S>(
    ws_stream: &mut WebSocketStream<S>,
    transport: &mut NoiseTransport,
) -> Result<Vec<u8>, TransportError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let msg = ws_stream
            .next()
            .await
            .ok_or(TransportError::ConnectionClosed)?
            .map_err(Box::new)?;
        match msg {
            Message::Binary(b) => return Ok(transport.decrypt(&b)?),
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => return Err(TransportError::ConnectionClosed),
            _ => return Err(TransportError::UnexpectedFrameType),
        }
    }
}

/// `receive_encrypted` com timeout.
pub async fn receive_encrypted_timeout<S>(
    ws_stream: &mut WebSocketStream<S>,
    transport: &mut NoiseTransport,
    timeout: std::time::Duration,
) -> Result<Vec<u8>, TransportError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match tokio::time::timeout(timeout, receive_encrypted(ws_stream, transport)).await {
        Ok(r) => r,
        Err(_) => Err(TransportError::Timeout),
    }
}

/// Encerramento limpo: envia um frame Close e descarta o stream.
pub async fn close_gracefully<S>(ws_stream: &mut WebSocketStream<S>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ws_stream.close(None)).await;
}

#[cfg(test)]
mod url_policy_tests {
    use super::*;

    #[test]
    fn aceita_wss_e_ws_so_em_loopback() {
        for ok in ["wss://relay.example.org", "WSS://x.y:443/p", "ws://127.0.0.1:9443", "ws://localhost:1", "ws://[::1]:9443"] {
            assert!(check_url_policy(ok, false).is_ok(), "{ok}");
        }
    }

    #[test]
    fn recusa_ws_remoto_outros_esquemas_e_hosts_que_so_parecem_loopback() {
        for bad in [
            "ws://relay.example.org",
            "ws://192.168.1.10:9443",
            "http://x",
            "relay.example.org",
            "ws://127.0.0.1.evil.com:9443",
            "ws://localhost.evil.org",
            "ws://127.0.0.1@evil.com/",
        ] {
            assert!(check_url_policy(bad, false).is_err(), "{bad}");
        }
        assert!(check_url_policy("ws://192.168.1.10:9443", true).is_ok(), "opt-in explicito");
    }
}
