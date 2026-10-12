//! Cliente do relay com RECONNECT, timeouts e deteccao de sessao invalida.
//!
//! O estado criptografico (ratchet, prekeys, dedup) vive no storage do
//! `QtClient`, NAO nesta ligacao: perder a ligacao nunca perde estado
//! criptografico. Cada reconnect faz um handshake Noise novo + autenticacao.
//!
//! Semantica de retry: um pedido que falha por rede e repetido numa ligacao
//! nova ate `max_retries`. Como a entrega e at-least-once, um `SendMessage`
//! pode chegar duplicado - o destinatario descarta duplicados (replay
//! protection do `QtClient::decrypt`).

use crate::protocol::*;
use qt_core::primitives::SigningKeyPair;
use qt_transport::ws_transport::{self, ClientOptions, ClientWsStream, TransportError};
use qt_transport::NoiseTransport;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct RelayConfig {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub max_retries: u32,
    pub backoff_base: Duration,
    /// Pinning da chave estatica Noise do relay (recomendado em producao).
    pub expected_server_static: Option<Vec<u8>>,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(15),
            max_retries: 3,
            backoff_base: Duration::from_millis(200),
            expected_server_static: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum RelayError {
    #[error("relay indisponivel")]
    Unavailable,
    #[error("timeout")]
    Timeout,
    /// A chave do relay nao corresponde ao pin: NAO repetir (possivel MITM).
    #[error("sessao invalida: identidade do relay inesperada")]
    SessionInvalid,
    #[error("autenticacao rejeitada pelo relay")]
    AuthRejected,
    #[error("rejeitado pelo relay: {0}")]
    Rejected(String),
    #[error("resposta inesperada do relay")]
    Protocol,
}

struct Conn {
    ws: ClientWsStream,
    noise: NoiseTransport,
}

pub struct RelayClient {
    url: String,
    noise_private: Vec<u8>,
    signing: SigningKeyPair,
    cfg: RelayConfig,
    conn: Option<Conn>,
    user_id: UserId,
}

impl RelayClient {
    /// Liga, faz o handshake Noise e autentica. `signing_private` e a chave
    /// Ed25519 da identidade (o `user_id` no relay e `hex(signing_public)`).
    pub async fn connect(
        url: &str,
        noise_private: Vec<u8>,
        signing_private: [u8; 32],
        cfg: RelayConfig,
    ) -> Result<Self, RelayError> {
        let signing = SigningKeyPair::from_bytes(&signing_private);
        let user_id = user_id_from_signing_public(&signing.verifying_key.to_bytes());
        let mut c = RelayClient { url: url.to_string(), noise_private, signing, cfg, conn: None, user_id };
        c.ensure_connected().await?;
        Ok(c)
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }

    async fn ensure_connected(&mut self) -> Result<(), RelayError> {
        if self.conn.is_some() {
            return Ok(());
        }
        let opts = ClientOptions {
            connect_timeout: self.cfg.connect_timeout,
            expected_server_static: self.cfg.expected_server_static.clone(),
        };
        let (mut ws, mut noise) = ws_transport::client_connect_with(&self.url, &self.noise_private, &opts)
            .await
            .map_err(|e| match e {
                TransportError::ServerKeyMismatch => RelayError::SessionInvalid,
                TransportError::Timeout => RelayError::Timeout,
                _ => RelayError::Unavailable,
            })?;
        // Autenticacao ligada ao hash do handshake desta sessao.
        let mut msg = AUTH_CONTEXT.to_vec();
        msg.extend_from_slice(noise.handshake_hash());
        let sig = self.signing.sign(&msg);
        let auth = ClientMessage::Authenticate {
            signing_public: self.signing.verifying_key.to_bytes().to_vec(),
            signature: sig.to_bytes().to_vec(),
        };
        let resp = Self::roundtrip(&mut ws, &mut noise, &auth, self.cfg.request_timeout).await?;
        match resp {
            ServerMessage::Ack => {
                self.conn = Some(Conn { ws, noise });
                Ok(())
            }
            ServerMessage::Error { .. } => Err(RelayError::AuthRejected),
            _ => Err(RelayError::Protocol),
        }
    }

    async fn roundtrip(
        ws: &mut ClientWsStream,
        noise: &mut NoiseTransport,
        msg: &ClientMessage,
        timeout: Duration,
    ) -> Result<ServerMessage, RelayError> {
        let bytes = serialize_client_message(msg);
        ws_transport::send_encrypted(ws, noise, &bytes).await.map_err(|_| RelayError::Unavailable)?;
        let resp = ws_transport::receive_encrypted_timeout(ws, noise, timeout).await.map_err(|e| match e {
            TransportError::Timeout => RelayError::Timeout,
            _ => RelayError::Unavailable,
        })?;
        deserialize_server_message(&resp).map_err(|_| RelayError::Protocol)
    }

    /// Envia um pedido com reconnect automatico + backoff exponencial.
    /// `ServerMessage::Error` e devolvido como `Ok` (o relay respondeu).
    pub async fn request(&mut self, msg: ClientMessage) -> Result<ServerMessage, RelayError> {
        let mut last = RelayError::Unavailable;
        for attempt in 0..=self.cfg.max_retries {
            if attempt > 0 {
                let delay = self.cfg.backoff_base * 2u32.pow((attempt - 1).min(6));
                tokio::time::sleep(delay).await;
            }
            match self.ensure_connected().await {
                Ok(()) => {}
                // Erros definitivos: nao insistir.
                Err(e @ (RelayError::SessionInvalid | RelayError::AuthRejected | RelayError::Protocol)) => return Err(e),
                Err(e) => {
                    last = e;
                    continue;
                }
            }
            let timeout = self.cfg.request_timeout;
            let Some(conn) = self.conn.as_mut() else { continue };
            match Self::roundtrip(&mut conn.ws, &mut conn.noise, &msg, timeout).await {
                Ok(r) => return Ok(r),
                Err(RelayError::Protocol) => {
                    self.conn = None;
                    return Err(RelayError::Protocol);
                }
                Err(e) => {
                    // Apos qualquer falha o canal Noise pode ter ficado
                    // dessincronizado (resposta tardia): descartar e re-ligar.
                    self.conn = None;
                    last = e;
                }
            }
        }
        Err(last)
    }

    fn expect_ack(r: ServerMessage) -> Result<(), RelayError> {
        match r {
            ServerMessage::Ack => Ok(()),
            ServerMessage::Error { reason } => Err(RelayError::Rejected(reason)),
            _ => Err(RelayError::Protocol),
        }
    }

    pub async fn register_bundle(&mut self, bundle_bytes: Vec<u8>) -> Result<(), RelayError> {
        let uid = self.user_id.clone();
        Self::expect_ack(self.request(ClientMessage::RegisterPreKeyBundle { user_id: uid, bundle_bytes }).await?)
    }

    pub async fn fetch_bundle(&mut self, user_id: &str) -> Result<Option<Vec<u8>>, RelayError> {
        match self.request(ClientMessage::FetchPreKeyBundle { user_id: user_id.to_string() }).await? {
            ServerMessage::PreKeyBundle { bundle_bytes } => Ok(Some(bundle_bytes)),
            ServerMessage::PreKeyBundleNotFound => Ok(None),
            ServerMessage::Error { reason } => Err(RelayError::Rejected(reason)),
            _ => Err(RelayError::Protocol),
        }
    }

    pub async fn send_message(&mut self, to: &str, sealed_from: Vec<u8>, ciphertext: Vec<u8>) -> Result<(), RelayError> {
        Self::expect_ack(self.request(ClientMessage::SendMessage { to: to.to_string(), sealed_from, ciphertext }).await?)
    }

    /// Proxima pagina de mensagens (sem as apagar do relay). Processar,
    /// persistir e so depois chamar `ack`.
    pub async fn fetch_messages(&mut self) -> Result<Vec<DeliveredMessage>, RelayError> {
        let uid = self.user_id.clone();
        match self.request(ClientMessage::FetchMessages { user_id: uid }).await? {
            ServerMessage::Messages { messages } => Ok(messages),
            ServerMessage::Error { reason } => Err(RelayError::Rejected(reason)),
            _ => Err(RelayError::Protocol),
        }
    }

    pub async fn ack(&mut self, ids: Vec<u64>) -> Result<(), RelayError> {
        let uid = self.user_id.clone();
        Self::expect_ack(self.request(ClientMessage::AckMessages { user_id: uid, ids }).await?)
    }

    /// Encerramento limpo (frame Close). Pode voltar a usar-se `request`
    /// depois: re-liga automaticamente.
    pub async fn close(&mut self) {
        if let Some(mut c) = self.conn.take() {
            ws_transport::close_gracefully(&mut c.ws).await;
        }
    }
}

/// Conveniencia: liga (chave Noise efemera), autentica com `signing_private`,
/// envia UM pedido e fecha. Sem retries. Util para testes e ferramentas.
pub async fn one_shot(
    url: &str,
    signing_private: [u8; 32],
    msg: ClientMessage,
) -> Result<ServerMessage, RelayError> {
    let noise = qt_transport::generate_static_keypair().map_err(|_| RelayError::Protocol)?;
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    let mut c = RelayClient::connect(url, noise.private, signing_private, cfg).await?;
    let r = c.request(msg).await;
    c.close().await;
    r
}
