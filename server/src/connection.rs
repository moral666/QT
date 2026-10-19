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
use crate::blobs::{b64, BlobError};
use crate::stats::{self, STATS};
use base64::Engine;
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
    /// Ligacoes por autenticar ao mesmo tempo, em todo o relay (0 = sem limite).
    pub max_unauthenticated: usize,
    /// Ligacoes autenticadas por conta; acima disto fecha-se a mais antiga (0 = sem limite).
    pub max_connections_per_user: usize,
    /// Exigir prova de trabalho (`qt_core::pow`) ao primeiro bundle de uma conta.
    pub require_pow: bool,
    /// Envelopes enviados por conta (`SendMessage`).
    pub send_rate: crate::limits::Rate,
    /// Pedidos de bundle por conta (`FetchPreKeyBundle`).
    pub fetch_bundle_rate: crate::limits::Rate,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            // 5 + 5 s (antes 10 + 10): as apps fazem os dois em menos de 1 s, e uma
            // ligacao que nunca se autentica ocupa a vaga menos tempo.
            handshake_timeout: Duration::from_secs(5),
            auth_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(300),
            max_unauthenticated: 2000,
            max_connections_per_user: 8,
            require_pow: true,
            // 300 seguidos e 300 por minuto: chega para conversas e grupos muito ativos
            // (e para avisar todos os contactos ao abrir a app), nao para spam em massa.
            send_rate: crate::limits::Rate { burst: 300, per_minute: 300 },
            fetch_bundle_rate: crate::limits::Rate { burst: 60, per_minute: 60 },
        }
    }
}

#[derive(Debug, Error)]
pub enum ConnectionError {
    #[error("erro de transporte: {0}")]
    Transport(#[from] TransportError),
    #[error("autenticacao falhou")]
    AuthFailed,
    #[error("demasiadas ligacoes por autenticar")]
    Busy,
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
    // Sem o algoritmo de Nagle: as respostas sao pequenas e seguidas (ex.: um
    // `BlobStatus` por cada bloco de um anexo) e o Nagle segurava cada uma a espera
    // do ACK do cliente. Com ele, enviar 25 MB demorava ~9x mais (docs/performance.md).
    let _ = tcp_stream.set_nodelay(true);
    // Vaga de "por autenticar": libertada logo que a autenticacao passa (ou a ligacao cai).
    let Some(unauthenticated) = crate::limits::UnauthenticatedSlot::try_acquire(cfg.max_unauthenticated) else {
        stats::inc(&STATS.rejected_unauthenticated);
        return Err(ConnectionError::Busy);
    };
    let (mut ws, mut noise) =
        match ws_transport::server_accept_with(tcp_stream, &server_static_private, cfg.handshake_timeout).await {
            Ok(x) => x,
            Err(e) => {
                stats::inc(&STATS.handshake_failed);
                return Err(e.into());
            }
        };

    // ---------- Autenticacao obrigatoria (1.a mensagem) ----------
    let first = ws_transport::receive_encrypted_timeout(&mut ws, &mut noise, cfg.auth_timeout).await?;
    let user_id = match deserialize_client_message(&first) {
        Ok(ClientMessage::Authenticate { signing_public, signature }) => {
            match verify_auth(&signing_public, &signature, noise.handshake_hash()) {
                true => user_id_from_signing_public(&signing_public),
                false => {
                    stats::inc(&STATS.auth_failed);
                    reply(&mut ws, &mut noise, &err("autenticacao invalida")).await?;
                    ws_transport::close_gracefully(&mut ws).await;
                    return Err(ConnectionError::AuthFailed);
                }
            }
        }
        _ => {
            stats::inc(&STATS.auth_failed);
            reply(&mut ws, &mut noise, &err("autenticacao necessaria")).await?;
            ws_transport::close_gracefully(&mut ws).await;
            return Err(ConnectionError::AuthFailed);
        }
    };
    stats::inc(&STATS.auth_ok);
    drop(unauthenticated);
    // No maximo `max_connections_per_user` por conta: as mais antigas fecham.
    let (slot, closed) = crate::limits::UserSlot::register(&user_id, cfg.max_connections_per_user);
    stats::add(&STATS.closed_oldest, closed as u64);
    reply(&mut ws, &mut noise, &ServerMessage::Ack).await?;

    // ---------- Loop principal ----------
    // Pedido que chegou a meio de uma espera (WaitMessages): tratado a seguir.
    let mut pending: Option<Vec<u8>> = None;
    let mut probe = IdaProbe::default();
    let main_loop = async {
        loop {
            let payload = match pending.take() {
                Some(p) => p,
                None => match next_request(&mut ws, &mut noise, cfg.idle_timeout, &mut probe).await {
                    Some(p) => p,
                    // Fecho normal, timeout de inatividade ou frame invalido: encerra a sessao.
                    None => break,
                },
            };
            let mut sent_message = false;
            let response = match deserialize_client_message(&payload) {
                Ok(ClientMessage::WaitMessages { user_id: asked, timeout_secs }) => {
                    match wait_messages(&store, &user_id, &asked, timeout_secs, &mut ws, &mut noise, &mut probe).await {
                        WaitEnd::Reply(r) => r,
                        WaitEnd::ReplyThen(r, next) => {
                            pending = Some(next);
                            r
                        }
                        WaitEnd::Closed => break,
                    }
                }
                Ok(m) => {
                    sent_message = matches!(m, ClientMessage::SendMessage { .. });
                    handle_client_message(&store, &user_id, m, cfg).await
                }
                Err(_) => {
                    stats::inc(&STATS.invalid_requests);
                    err("mensagem invalida")
                }
            };
            if reply(&mut ws, &mut noise, &response).await.is_err() {
                break;
            }
            // Acabou de enviar uma mensagem: mede a ida e volta ate este telemovel.
            if sent_message && matches!(response, ServerMessage::Ack) {
                if let Some(ping) = probe.start() {
                    if ws_transport::send_ping(&mut ws, ping).await.is_err() {
                        break;
                    }
                }
            }
        }
    };
    // Termina com o loop ou quando uma ligacao mais nova da mesma conta ocupa a vaga.
    tokio::select! {
        _ = main_loop => {}
        _ = slot.must_close() => {}
    }
    ws_transport::close_gracefully(&mut ws).await;
    drop(slot);
    Ok(())
}

/// Mede o tempo de ida do telemovel ate ao relay: logo depois de aceitar uma
/// mensagem (`SendMessage`), o relay manda um ping nesta ligacao e espera pelo
/// pong. A ida e metade dessa ida e volta (ver `latency::ida_sample`). Um ping de
/// cada vez por ligacao; o conteudo e so um contador, para reconhecer o pong.
#[derive(Default)]
struct IdaProbe {
    counter: u64,
    sent: Option<(u64, std::time::Instant)>,
}

impl IdaProbe {
    /// Conteudo do ping a enviar, ou `None` se ainda ha um a espera de resposta.
    fn start(&mut self) -> Option<Vec<u8>> {
        let now = std::time::Instant::now();
        if let Some((_, at)) = self.sent {
            if now.duration_since(at) < crate::latency::MAX_ROUND_TRIP {
                return None;
            }
        }
        self.counter += 1;
        self.sent = Some((self.counter, now));
        Some(format!("qt-ida-{}", self.counter).into_bytes())
    }

    /// Chegou um pong: se for o do nosso ping, conta a ida e volta.
    fn pong(&mut self, payload: &[u8]) {
        if let Some((n, at)) = self.sent {
            if payload == format!("qt-ida-{n}").as_bytes() {
                self.sent = None;
                crate::latency::ida_sample(at.elapsed());
            }
        }
    }
}

/// Proximo pedido do cliente (ja decifrado). Pelo caminho responde a pings e
/// regista pongs. `None`: fecho, frame invalido ou `idle` sem pedidos.
async fn next_request(
    ws: &mut ws_transport::ServerWsStream,
    noise: &mut qt_transport::NoiseTransport,
    idle: Duration,
    probe: &mut IdaProbe,
) -> Option<Vec<u8>> {
    let deadline = tokio::time::Instant::now() + idle;
    loop {
        match tokio::time::timeout_at(deadline, ws_transport::next_incoming(ws)).await {
            Ok(ws_transport::Incoming::Data(frame)) => return noise.decrypt(&frame).ok(),
            Ok(ws_transport::Incoming::Pong(p)) => probe.pong(&p),
            Ok(ws_transport::Incoming::Control) => {}
            Ok(ws_transport::Incoming::Closed) | Err(_) => return None,
        }
    }
}

fn signing_public_of(user_id: &str) -> Option<[u8; 32]> {
    if !is_valid_user_id(user_id) {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&user_id[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
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

enum WaitEnd {
    /// Responder e continuar a ler a ligacao.
    Reply(ServerMessage),
    /// Responder e depois tratar este pedido, que chegou a meio da espera (ja decifrado).
    ReplyThen(ServerMessage, Vec<u8>),
    /// A ligacao fechou (ou mandou um frame invalido) durante a espera.
    Closed,
}

/// `WaitMessages`: devolve a fila; se estiver vazia, espera ate chegar uma mensagem
/// para este utilizador ou ate `timeout_secs` (no maximo `waiters::MAX_WAIT_SECS`).
/// Durante a espera continua a ler a ligacao: responde logo a pings (a app Android
/// fecha a ligacao se o pong demorar) e, se chegar outro pedido, acaba a espera.
async fn wait_messages(
    store: &Store,
    me: &str,
    user_id: &str,
    timeout_secs: u64,
    ws: &mut ws_transport::ServerWsStream,
    noise: &mut qt_transport::NoiseTransport,
    probe: &mut IdaProbe,
) -> WaitEnd {
    if user_id != me {
        return WaitEnd::Reply(err("so podes ler a tua propria fila"));
    }
    stats::inc(&STATS.wait_requests);
    let wait = Duration::from_secs(timeout_secs.min(crate::waiters::MAX_WAIT_SECS));
    // Regista a espera ANTES de olhar para a fila: uma mensagem que chegue entre o
    // "fila vazia" e o inicio da espera acorda-nos na mesma (nao fica 25 s parada).
    let waiter = crate::waiters::Waiter::register(me);
    let notified = waiter.notify().notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    match store.peek_messages(user_id, MAX_PAGE_BYTES).await {
        Ok(raw) if !raw.is_empty() || wait.is_zero() => return WaitEnd::Reply(messages(raw)),
        Ok(_) => {}
        Err(e) => return WaitEnd::Reply(internal(e)),
    }
    stats::inc(&STATS.waits);
    let deadline = tokio::time::sleep(wait);
    tokio::pin!(deadline);
    let mut next = None;
    loop {
        tokio::select! {
            _ = &mut notified => break,
            _ = &mut deadline => break,
            incoming = ws_transport::next_incoming(ws) => match incoming {
                ws_transport::Incoming::Control => continue,
                ws_transport::Incoming::Pong(p) => probe.pong(&p),
                ws_transport::Incoming::Data(frame) => match noise.decrypt(&frame) {
                    Ok(p) => {
                        next = Some(p);
                        break;
                    }
                    Err(_) => return WaitEnd::Closed,
                },
                ws_transport::Incoming::Closed => return WaitEnd::Closed,
            },
        }
    }
    let reply = match store.peek_messages(user_id, MAX_PAGE_BYTES).await {
        Ok(raw) => messages(raw),
        Err(e) => internal(e),
    };
    match next {
        Some(p) => WaitEnd::ReplyThen(reply, p),
        None => WaitEnd::Reply(reply),
    }
}

/// Resposta `Messages` (de `FetchMessages` ou `WaitMessages`).
fn messages(raw: Vec<crate::store::QueuedMessage>) -> ServerMessage {
    stats::inc(&STATS.fetches);
    stats::add(&STATS.delivered, raw.len() as u64);
    ServerMessage::Messages {
        messages: raw
            .into_iter()
            .map(|m| DeliveredMessage { id: m.id, sealed_from: m.sealed_from, ciphertext: m.ciphertext })
            .collect(),
    }
}

/// Erros internos nunca chegam ao cliente em detalhe.
fn internal(_e: StoreError) -> ServerMessage {
    stats::inc(&STATS.internal_errors);
    err("erro interno")
}

async fn handle_client_message(store: &Store, me: &str, msg: ClientMessage, cfg: &ServerConfig) -> ServerMessage {
    match msg {
        ClientMessage::Authenticate { .. } => err("ja autenticado"),
        ClientMessage::RegisterPreKeyBundle { user_id, bundle_bytes, pow, one_time_pre_keys } => {
            if user_id != me {
                return err("so podes registar o teu proprio bundle");
            }
            if bundle_bytes.is_empty() || bundle_bytes.len() > MAX_BUNDLE {
                return err("bundle com tamanho invalido");
            }
            // O bundle tem de pertencer a identidade AUTENTICADA: a chave Ed25519
            // embebida e a da ligacao, e a assinatura (que cobre a identidade
            // X25519, a signed pre-key e a prekey ML-KEM) tem de validar.
            let Some(signing) = signing_public_of(me) else { return err("identidade invalida") };
            if let Err(e) = qt_core::identity::verify_published_bundle(&bundle_bytes, &signing) {
                return err(match e {
                    qt_core::identity::IdentityError::WrongOwner => "o bundle nao pertence a esta identidade",
                    qt_core::identity::IdentityError::BadSignature => "assinatura do bundle invalida",
                    qt_core::identity::IdentityError::BadFormat => "bundle com formato invalido",
                });
            }
            if let Some(keys) = &one_time_pre_keys {
                if keys.len() > crate::protocol::MAX_OTPKS
                    || keys.iter().any(|k| k.len() != crate::protocol::OTPK_LEN || k[..4] == [0; 4])
                {
                    return err("one-time prekeys invalidas");
                }
            }
            // Conta nova (ou que expirou): so com prova de trabalho valida.
            if cfg.require_pow {
                match store.get_bundle(me).await {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        if !pow.is_some_and(|n| qt_core::pow::pow_verify(&signing, n, qt_core::pow::POW_BITS)) {
                            stats::inc(&STATS.pow_rejected);
                            return err(crate::protocol::POW_REQUIRED);
                        }
                    }
                    Err(e) => return internal(e),
                }
            }
            match store.register_bundle(user_id, bundle_bytes).await {
                Ok(()) => {
                    stats::inc(&STATS.bundles_registered);
                    if let Some(keys) = one_time_pre_keys {
                        if let Err(e) = store.replace_one_time_prekeys(me, &keys).await {
                            return internal(e);
                        }
                    }
                    ServerMessage::Ack
                }
                Err(e) => internal(e),
            }
        }
        ClientMessage::FetchPreKeyBundle { user_id } => {
            if !is_valid_user_id(&user_id) {
                return err("user_id invalido");
            }
            // Cada pedido gasta uma one-time prekey do contacto: sem limite, alguem
            // podia esvazia-las de proposito.
            if !crate::limits::allow(crate::limits::Limited::FetchBundle, me, cfg.fetch_bundle_rate) {
                stats::inc(&STATS.rate_limited);
                return err(crate::protocol::RATE_LIMITED);
            }
            match store.get_bundle(&user_id).await {
                Ok(Some(bundle_bytes)) => {
                    stats::inc(&STATS.bundles_fetched);
                    let one_time_pre_key = match store.pop_one_time_prekey(&user_id).await {
                        Ok(k) => k.filter(|k| k.len() == crate::protocol::OTPK_LEN),
                        Err(e) => return internal(e),
                    };
                    if one_time_pre_key.is_some() {
                        stats::inc(&STATS.otpks_served);
                    }
                    ServerMessage::PreKeyBundle { bundle_bytes, one_time_pre_key }
                }
                Ok(None) => {
                    stats::inc(&STATS.bundles_not_found);
                    ServerMessage::PreKeyBundleNotFound
                }
                Err(e) => internal(e),
            }
        }
        ClientMessage::SendMessage { to, sealed_from, ciphertext, silent } => {
            if !is_valid_user_id(&to) {
                return err("destinatario invalido");
            }
            if sealed_from.len() > MAX_SEALED_FROM || ciphertext.is_empty() || ciphertext.len() > MAX_CIPHERTEXT {
                return err("mensagem com tamanho invalido");
            }
            if !crate::limits::allow(crate::limits::Limited::Send, me, cfg.send_rate) {
                stats::inc(&STATS.rate_limited);
                return err(crate::protocol::RATE_LIMITED);
            }
            let recipient = to.clone();
            match store.enqueue_message(to, sealed_from, ciphertext).await {
                Ok(id) => {
                    stats::inc(&STATS.sends);
                    crate::latency::queued(id);
                    // Entrega imediata: acorda o destinatario se estiver a espera (WaitMessages).
                    crate::waiters::wake(&recipient);
                    // App fechada (ninguem a espera): campainha pela Google (FCM), vazia.
                    // Recibos, reacoes e presenca (`silent`) nao tocam.
                    if !silent {
                        crate::push::after_enqueue(store, &recipient).await;
                    }
                    ServerMessage::Ack
                }
                Err(StoreError::QueueFull) => {
                    stats::inc(&STATS.queue_full);
                    err("fila do destinatario cheia")
                }
                Err(StoreError::RecipientDeleted) => err(crate::protocol::RECIPIENT_DELETED),
                Err(e) => internal(e),
            }
        }
        ClientMessage::DeleteAccount { user_id } => {
            if user_id != me {
                return err("so podes apagar a tua propria conta");
            }
            match store.delete_account(&user_id).await {
                Ok(()) => {
                    stats::inc(&STATS.accounts_deleted);
                    ServerMessage::Ack
                }
                Err(e) => internal(e),
            }
        }
        ClientMessage::RegisterPushToken { user_id, token } => {
            if user_id != me {
                return err("so podes registar o teu proprio token");
            }
            if !token.is_empty() && !crate::protocol::is_valid_push_token(&token) {
                return err("token invalido");
            }
            match store.set_push_token(&user_id, (!token.is_empty()).then_some(token.as_str())).await {
                Ok(()) => {
                    stats::inc(&STATS.push_registered);
                    ServerMessage::Ack
                }
                Err(e) => internal(e),
            }
        }
        ClientMessage::FetchMessages { user_id } => {
            if user_id != me {
                return err("so podes ler a tua propria fila");
            }
            match store.peek_messages(&user_id, MAX_PAGE_BYTES).await {
                Ok(raw) => messages(raw),
                Err(e) => internal(e),
            }
        }
        // Tratado no loop da ligacao (precisa dela para responder a pings durante a espera).
        ClientMessage::WaitMessages { user_id, .. } => {
            if user_id != me {
                return err("so podes ler a tua propria fila");
            }
            match store.peek_messages(&user_id, MAX_PAGE_BYTES).await {
                Ok(raw) => messages(raw),
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
            match store.ack_messages_ids(&user_id, &ids).await {
                Ok(removed) => {
                    stats::add(&STATS.acked, removed.len() as u64);
                    crate::latency::acked(&removed);
                    ServerMessage::Ack
                }
                Err(e) => internal(e),
            }
        }
        ClientMessage::BlobCreate { size } => match store.blob_create(me, size).await {
            Ok((blob_id, token)) => {
                stats::inc(&STATS.blob_uploads);
                ServerMessage::BlobCreated { blob_id, token }
            }
            Err(e) => blob_err(e),
        },
        ClientMessage::BlobPut { blob_id, token, offset, data } => {
            match store.blob_put(&blob_id, &token, offset, &data).await {
                Ok(s) => ServerMessage::BlobStatus { size: s.size, received: s.received, complete: s.complete },
                Err(e) => blob_err(e),
            }
        }
        ClientMessage::BlobInfo { blob_id } => match store.blob_info(&blob_id).await {
            Ok(s) => ServerMessage::BlobStatus { size: s.size, received: s.received, complete: s.complete },
            Err(e) => blob_err(e),
        },
        ClientMessage::BlobGet { blob_id, offset } => match store.blob_get(&blob_id, offset).await {
            Ok((size, data)) => {
                if offset == 0 {
                    stats::inc(&STATS.blob_downloads);
                }
                ServerMessage::BlobData { offset, size, data: b64().encode(data) }
            }
            Err(e) => blob_err(e),
        },
        ClientMessage::BlobDelete { blob_id } => match store.blob_delete(&blob_id).await {
            Ok(()) => ServerMessage::Ack,
            Err(e) => blob_err(e),
        },
    }
}

/// Erros dos anexos: mensagens curtas, sem detalhes internos.
fn blob_err(e: BlobError) -> ServerMessage {
    match e {
        BlobError::NotFound => ServerMessage::BlobNotFound,
        BlobError::Io(_) | BlobError::Store(_) => {
            stats::inc(&STATS.internal_errors);
            err("erro interno")
        }
        other => err(&other.to_string()),
    }
}
