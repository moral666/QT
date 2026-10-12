//! Trata uma ligacao de cliente: handshake Noise (via `transport/`),
//! AUTENTICACAO obrigatoria (assinatura Ed25519 ligada ao hash do handshake)
//! e depois o loop de `ClientMessage`s -> `ServerMessage`s, sempre dentro do
//! canal Noise.
//!
//! Garantias deste modulo:
//! - nenhuma operacao sobre filas/bundles sem autenticacao previa;
//! - um utilizador so le/apaga a PROPRIA fila e so regista o PROPRIO bundle;
//! - timeouts de handshake, autenticacao e inatividade;
//! - limites de tamanho por campo; `user_id` validado (64 hex);
//! - erros enviados ao cliente sao genericos (nunca detalhes do Redis).
//! - qualquer frame que nao decifre (corrompido/repetido/fora de ordem)
//!   encerra a sessao.

use crate::protocol::{
    deserialize_client_message, is_valid_user_id, serialize_server_message,
    user_id_from_signing_public, ClientMessage, DeliveredMessage, ServerMessage, AUTH_CONTEXT,
};
use crate::store::{Store, StoreError};
use ed25519_dalek::{Signature, VerifyingKey};
use qt_core::primitives::verify_signature;
use qt_transport::ws_transport::{self, TransportError};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpStream;

/// Maximo de payload estimado (JSON) por resposta `Messages` - cabe num frame Noise.
const MAX_PAGE_BYTES: usize = 40_000;
pub const MAX_SEALED_FROM: usize = 1024;
pub const MAX_CIPHERTEXT: usize = 14_000;
pub const MAX_BUNDLE: usize = 24_000;
pub const MAX_ACK_IDS: usize = 200;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub handshake_timeout: Duration,
    pub auth_timeout: Duration,
    pub idle_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            handshake_timeout: Duration::from_secs(10),
            auth_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(300),
        }
    }
}

#[derive(Debug, Error)]
pub enum ConnectionError {
    #[error("erro de transporte: {0}")]
    Transport(#[from] TransportError),
    #[error("autenticacao falhou")]
    AuthFailed,
}

pub async fn handle_connection(
    tcp_stream: TcpStream,
    store: Arc<Store>,
    server_static_private: Vec<u8>,
) -> Result<(), ConnectionError> {
    handle_connection_with(tcp_stream, store, server_static_private, &ServerConfig::default()).await
}

pub async fn handle_connection_with(
    tcp_stream: TcpStream,
    store: Arc<Store>,
    server_static_private: Vec<u8>,
    cfg: &ServerConfig,
) -> Result<(), ConnectionError> {
    let (mut ws, mut noise) =
        ws_transport::server_accept_with(tcp_stream, &server_static_private, cfg.handshake_timeout).await?;

    // ---------- Autenticacao obrigatoria (1.a mensagem) ----------
    let first = ws_transport::receive_encrypted_timeout(&mut ws, &mut noise, cfg.auth_timeout).await?;
    let user_id = match deserialize_client_message(&first) {
        Ok(ClientMessage::Authenticate { signing_public, signature }) => {
            match verify_auth(&signing_public, &signature, noise.handshake_hash()) {
                true => user_id_from_signing_public(&signing_public),
                false => {
                    reply(&mut ws, &mut noise, &err("autenticacao invalida")).await?;
                    ws_transport::close_gracefully(&mut ws).await;
                    return Err(ConnectionError::AuthFailed);
                }
            }
        }
        _ => {
            reply(&mut ws, &mut noise, &err("autenticacao necessaria")).await?;
            ws_transport::close_gracefully(&mut ws).await;
            return Err(ConnectionError::AuthFailed);
        }
    };
    reply(&mut ws, &mut noise, &ServerMessage::Ack).await?;

    // ---------- Loop principal ----------
    loop {
        let payload = match ws_transport::receive_encrypted_timeout(&mut ws, &mut noise, cfg.idle_timeout).await {
            Ok(p) => p,
            // Fecho normal, timeout de inatividade ou frame invalido: encerra a sessao.
            Err(_) => break,
        };
        let response = match deserialize_client_message(&payload) {
            Ok(m) => handle_client_message(&store, &user_id, m).await,
            Err(_) => err("mensagem invalida"),
        };
        if reply(&mut ws, &mut noise, &response).await.is_err() {
            break;
        }
    }
    ws_transport::close_gracefully(&mut ws).await;
    Ok(())
}

fn verify_auth(signing_public: &[u8], signature: &[u8], handshake_hash: &[u8]) -> bool {
    let Ok(pk): Result<[u8; 32], _> = signing_public.try_into() else { return false };
    let Ok(sig): Result<[u8; 64], _> = signature.try_into() else { return false };
    let Ok(vk) = VerifyingKey::from_bytes(&pk) else { return false };
    let mut msg = AUTH_CONTEXT.to_vec();
    msg.extend_from_slice(handshake_hash);
    verify_signature(&vk, &msg, &Signature::from_bytes(&sig)).is_ok()
}

async fn reply(
    ws: &mut ws_transport::ServerWsStream,
    noise: &mut qt_transport::NoiseTransport,
    msg: &ServerMessage,
) -> Result<(), TransportError> {
    let bytes = serialize_server_message(msg);
    ws_transport::send_encrypted(ws, noise, &bytes).await
}

fn err(reason: &str) -> ServerMessage {
    ServerMessage::Error { reason: reason.to_string() }
}

/// Erros internos nunca chegam ao cliente em detalhe.
fn internal(_e: StoreError) -> ServerMessage {
    err("erro interno")
}

async fn handle_client_message(store: &Store, me: &str, msg: ClientMessage) -> ServerMessage {
    match msg {
        ClientMessage::Authenticate { .. } => err("ja autenticado"),
        ClientMessage::RegisterPreKeyBundle { user_id, bundle_bytes } => {
            if user_id != me {
                return err("so podes registar o teu proprio bundle");
            }
            if bundle_bytes.is_empty() || bundle_bytes.len() > MAX_BUNDLE {
                return err("bundle com tamanho invalido");
            }
            match store.register_bundle(user_id, bundle_bytes).await {
                Ok(()) => ServerMessage::Ack,
                Err(e) => internal(e),
            }
        }
        ClientMessage::FetchPreKeyBundle { user_id } => {
            if !is_valid_user_id(&user_id) {
                return err("user_id invalido");
            }
            match store.get_bundle(&user_id).await {
                Ok(Some(bundle_bytes)) => ServerMessage::PreKeyBundle { bundle_bytes },
                Ok(None) => ServerMessage::PreKeyBundleNotFound,
                Err(e) => internal(e),
            }
        }
        ClientMessage::SendMessage { to, sealed_from, ciphertext } => {
            if !is_valid_user_id(&to) {
                return err("destinatario invalido");
            }
            if sealed_from.len() > MAX_SEALED_FROM || ciphertext.is_empty() || ciphertext.len() > MAX_CIPHERTEXT {
                return err("mensagem com tamanho invalido");
            }
            match store.enqueue_message(to, sealed_from, ciphertext).await {
                Ok(()) => ServerMessage::Ack,
                Err(StoreError::QueueFull) => err("fila do destinatario cheia"),
                Err(e) => internal(e),
            }
        }
        ClientMessage::FetchMessages { user_id } => {
            if user_id != me {
                return err("so podes ler a tua propria fila");
            }
            match store.peek_messages(&user_id, MAX_PAGE_BYTES).await {
                Ok(raw) => ServerMessage::Messages {
                    messages: raw
                        .into_iter()
                        .map(|m| DeliveredMessage { id: m.id, sealed_from: m.sealed_from, ciphertext: m.ciphertext })
                        .collect(),
                },
                Err(e) => internal(e),
            }
        }
        ClientMessage::AckMessages { user_id, ids } => {
            if user_id != me {
                return err("so podes confirmar a tua propria fila");
            }
            if ids.len() > MAX_ACK_IDS {
                return err("demasiados ids");
            }
            match store.ack_messages(&user_id, &ids).await {
                Ok(_) => ServerMessage::Ack,
                Err(e) => internal(e),
            }
        }
    }
}
