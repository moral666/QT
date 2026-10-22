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
    /// Partes de copias de recuperacao enviadas ou pedidas por conta (`BackupPut`, `BackupGet`).
    pub backup_rate: crate::limits::Rate,
    /// Conta do operador (`hex` da chave Ed25519 de `/etc/qt/operator.pub`): so ela faz
    /// pedidos `Admin*` (denuncias e banimentos). `None` = desligados.
    pub operator: Option<String>,
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
            // Uma copia tem ate 8 partes: chega para refazer a copia algumas vezes por minuto.
            backup_rate: crate::limits::Rate { burst: 40, per_minute: 20 },
            operator: None,
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
    // Conta banida pelo operador: recusada. Se o Redis falhar, deixa entrar (o resto dos
    // pedidos falha na mesma e uma avaria nao deve fechar o relay a toda a gente).
    if matches!(store.is_banned(&user_id).await, Ok(true)) {
        stats::inc(&STATS.auth_failed);
        reply(&mut ws, &mut noise, &err(crate::protocol::BANNED)).await?;
        ws_transport::close_gracefully(&mut ws).await;
        return Err(ConnectionError::AuthFailed);
    }
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
            // Mensagem aceite nesta volta (id interno da fila), para medir a ida.
            let mut sent_message = None;
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
                Ok(ClientMessage::SendMessage { to, sealed_from, ciphertext, silent, wake }) => {
                    let (r, id) = send_message(&store, &user_id, to, sealed_from, ciphertext, silent, wake, cfg).await;
                    sent_message = id;
                    r
                }
                Ok(m) => handle_client_message(&store, &user_id, m, cfg).await,
                Err(_) => {
                    stats::inc(&STATS.invalid_requests);
                    err("mensagem invalida")
                }
            };
            if reply(&mut ws, &mut noise, &response).await.is_err() {
                break;
            }
            // Acabou de enviar uma mensagem: mede a ida e volta ate este telemovel.
            if let Some(id) = sent_message {
                if let Some(ping) = probe.start(id) {
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
/// cada vez por ligacao; o conteudo e so um contador, para reconhecer o pong. As
/// mensagens enviadas enquanto um ping espera resposta ficam com essa mesma medicao.
#[derive(Default)]
struct IdaProbe {
    counter: u64,
    sent: Option<(u64, std::time::Instant)>,
    /// Ids (internos da fila) das mensagens a espera deste ping.
    ids: Vec<u64>,
}

/// Mensagens que partilham um ping, no maximo (protege a memoria).
const MAX_IDS_PER_PROBE: usize = 64;

impl IdaProbe {
    /// A mensagem `id` foi aceite. Devolve o conteudo do ping a enviar, ou `None` se ja
    /// ha um a espera de resposta (a mensagem fica com a medicao desse).
    fn start(&mut self, id: u64) -> Option<Vec<u8>> {
        let now = std::time::Instant::now();
        if let Some((_, at)) = self.sent {
            if now.duration_since(at) < crate::latency::MAX_ROUND_TRIP {
                if self.ids.len() < MAX_IDS_PER_PROBE {
                    self.ids.push(id);
                }
                return None;
            }
        }
        self.counter += 1;
        self.sent = Some((self.counter, now));
        self.ids = vec![id];
        Some(format!("qt-ida-{}", self.counter).into_bytes())
    }

    /// Chegou um pong: se for o do nosso ping, conta a ida e volta.
    fn pong(&mut self, payload: &[u8]) {
        if let Some((n, at)) = self.sent {
            if payload == format!("qt-ida-{n}").as_bytes() {
                self.sent = None;
                crate::latency::ida_sample(at.elapsed(), &std::mem::take(&mut self.ids));
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
            match store.is_banned(me).await {
                Ok(false) => {}
                Ok(true) => return err(crate::protocol::BANNED),
                Err(e) => return internal(e),
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
                    // A app abriu: a copia de recuperacao fica mais 90 dias.
                    let _ = store.renew_backup(me).await;
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
        ClientMessage::SendMessage { to, sealed_from, ciphertext, silent, wake } => {
            send_message(store, me, to, sealed_from, ciphertext, silent, wake, cfg).await.0
        }
        ClientMessage::DeleteAccount { user_id } => {
            if user_id != me {
                return err("so podes apagar a tua propria conta");
            }
            match store.delete_account(&user_id).await {
                Ok(()) => {
                    let _ = store.delete_backup(&user_id).await;
                    let _ = store.set_premium(&user_id, 0).await;
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
            // Guardar um token (ate 4 KB, 90 dias) so com conta registada: chaves
            // descartaveis enchiam o Redis. Apagar (token vazio) e sempre permitido.
            if !token.is_empty() {
                match store.is_registered(me).await {
                    Ok(true) => {}
                    Ok(false) => {
                        stats::inc(&STATS.not_registered);
                        return err(crate::protocol::NOT_REGISTERED);
                    }
                    Err(e) => return internal(e),
                }
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
        // Anexo novo so com conta registada: chaves descartaveis contornavam a quota
        // diaria por conta e enchiam o espaco dos anexos (2 GB) para toda a gente.
        ClientMessage::BlobCreate { size } => match store.is_registered(me).await {
            Ok(false) => {
                stats::inc(&STATS.not_registered);
                err(crate::protocol::NOT_REGISTERED)
            }
            Err(e) => internal(e),
            // Enviar fotos, GIFs, voz e ficheiros e pago (0.9.21); receber continua gratis.
            Ok(true) => match store.premium_allows(me).await {
                Err(e) => internal(e),
                Ok(false) => err(crate::protocol::PREMIUM_REQUIRED),
                Ok(true) => match store.blob_create(me, size).await {
                    Ok((blob_id, token)) => {
                        stats::inc(&STATS.blob_uploads);
                        ServerMessage::BlobCreated { blob_id, token }
                    }
                    Err(e) => blob_err(e),
                },
            },
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
        ClientMessage::SubmitReport { sealed } => {
            let len_ok = sealed.len() > 1 + 32 + 16 + 4 && sealed.len() <= qt_core::report::MAX_SEALED;
            if !len_ok || sealed[0] != qt_core::report::VERSION {
                return err("denuncia invalida");
            }
            match store.is_registered(me).await {
                Ok(true) => {}
                Ok(false) => {
                    stats::inc(&STATS.not_registered);
                    return err(crate::protocol::NOT_REGISTERED);
                }
                Err(e) => return internal(e),
            }
            match store.submit_report(me, sealed).await {
                Ok(_) => {
                    stats::inc(&STATS.reports);
                    ServerMessage::Ack
                }
                Err(StoreError::ReportLimit) => err(crate::protocol::REPORT_LIMIT),
                Err(StoreError::ReportsFull) => err("demasiadas denuncias, tenta mais tarde"),
                Err(e) => internal(e),
            }
        }
        ClientMessage::AdminListReports { .. }
        | ClientMessage::AdminGrantPremium { .. }
        | ClientMessage::AdminListPremium
        | ClientMessage::AdminPremiumEnforce { .. }
        | ClientMessage::AdminDeleteReport { .. }
        | ClientMessage::AdminBan { .. }
        | ClientMessage::AdminUnban { .. }
        | ClientMessage::AdminListBans
            if cfg.operator.as_deref() != Some(me) =>
        {
            err(crate::protocol::NOT_OPERATOR)
        }
        ClientMessage::AdminListReports { after } => match store.list_reports(after, MAX_PAGE_BYTES).await {
            Ok((reports, more)) => ServerMessage::Reports { reports, more },
            Err(e) => internal(e),
        },
        ClientMessage::AdminDeleteReport { id } => match store.delete_report(id).await {
            Ok(()) => ServerMessage::Ack,
            Err(e) => internal(e),
        },
        ClientMessage::AdminBan { user_id } => {
            if !is_valid_user_id(&user_id) || user_id == me {
                return err("user_id invalido");
            }
            match store.ban(&user_id).await {
                Ok(()) => {
                    let _ = store.delete_backup(&user_id).await;
                    crate::limits::UserSlot::close_all(&user_id);
                    stats::inc(&STATS.bans);
                    ServerMessage::Ack
                }
                Err(e) => internal(e),
            }
        }
        ClientMessage::AdminUnban { user_id } => {
            if !is_valid_user_id(&user_id) {
                return err("user_id invalido");
            }
            match store.unban(&user_id).await {
                Ok(()) => ServerMessage::Ack,
                Err(e) => internal(e),
            }
        }
        ClientMessage::AdminListBans => match store.list_bans().await {
            Ok(user_ids) => ServerMessage::Bans { user_ids },
            Err(e) => internal(e),
        },
        ClientMessage::BackupPut { lookup, generation, part, parts, data } => {
            use crate::protocol::{is_hex_of_len, BACKUP_PARTS_MAX, BACKUP_PART_MAX};
            if !is_hex_of_len(&lookup, 64) || !is_hex_of_len(&generation, 16) || parts == 0 || parts > BACKUP_PARTS_MAX || part >= parts {
                return err("copia invalida");
            }
            if data.len() > BACKUP_PART_MAX.div_ceil(3) * 4 {
                return err("copia invalida");
            }
            let Ok(bytes) = b64().decode(&data) else { return err("copia invalida") };
            if bytes.is_empty() || bytes.len() > BACKUP_PART_MAX {
                return err("copia invalida");
            }
            if !crate::limits::allow(crate::limits::Limited::Backup, me, cfg.backup_rate) {
                stats::inc(&STATS.rate_limited);
                return err(crate::protocol::RATE_LIMITED);
            }
            match store.is_registered(me).await {
                Ok(true) => {}
                Ok(false) => {
                    stats::inc(&STATS.not_registered);
                    return err(crate::protocol::NOT_REGISTERED);
                }
                Err(e) => return internal(e),
            }
            match store.backup_put(me, &lookup, &generation, part, parts, bytes).await {
                Ok(()) => {
                    if part + 1 == parts {
                        stats::inc(&STATS.backups_saved);
                    }
                    ServerMessage::Ack
                }
                Err(StoreError::BackupNotOwner) => err(crate::protocol::BACKUP_NOT_OWNER),
                Err(StoreError::BackupIncomplete) => err("copia incompleta"),
                Err(e) => internal(e),
            }
        }
        ClientMessage::BackupGet { lookup, part } => {
            if !crate::protocol::is_hex_of_len(&lookup, 64) {
                return err("copia invalida");
            }
            if !crate::limits::allow(crate::limits::Limited::Backup, me, cfg.backup_rate) {
                stats::inc(&STATS.rate_limited);
                return err(crate::protocol::RATE_LIMITED);
            }
            match store.backup_get(&lookup, part).await {
                Ok(Some((parts, data))) => {
                    if part == 0 {
                        stats::inc(&STATS.backups_restored);
                    }
                    ServerMessage::BackupPart { part, parts, data: b64().encode(data) }
                }
                Ok(None) => ServerMessage::BackupNotFound,
                Err(e) => internal(e),
            }
        }
        ClientMessage::BackupDelete => match store.delete_backup(me).await {
            Ok(()) => ServerMessage::Ack,
            Err(e) => internal(e),
        },
        ClientMessage::PremiumStatus => match (store.premium_until(me).await, store.premium_enforced().await) {
            (Ok(until_ms), Ok(enforced)) => ServerMessage::Premium { until_ms, enforced, active: until_ms > crate::store::now_ms() },
            (Err(e), _) | (_, Err(e)) => internal(e),
        },
        ClientMessage::AdminPremiumEnforce { on } => {
            if let Some(on) = on {
                if let Err(e) = store.set_premium_enforced(on).await {
                    return internal(e);
                }
            }
            match store.premium_enforced().await {
                Ok(on) => ServerMessage::PremiumEnforce { on },
                Err(e) => internal(e),
            }
        }
        ClientMessage::AdminGrantPremium { user_id, until_ms } => {
            if !is_valid_user_id(&user_id) {
                return err("user_id invalido");
            }
            match store.set_premium(&user_id, until_ms).await {
                Ok(()) => ServerMessage::Ack,
                Err(e) => internal(e),
            }
        }
        ClientMessage::AdminListPremium => match store.list_premium().await {
            Ok(entries) => ServerMessage::PremiumList { entries },
            Err(e) => internal(e),
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

/// `SendMessage`. Devolve tambem o id interno da fila quando a mensagem foi aceite.
#[allow(clippy::too_many_arguments)]
async fn send_message(
    store: &Store,
    me: &str,
    to: crate::protocol::UserId,
    sealed_from: Vec<u8>,
    ciphertext: Vec<u8>,
    silent: bool,
    wake: Option<crate::protocol::WakeKind>,
    cfg: &ServerConfig,
) -> (ServerMessage, Option<u64>) {
    if !is_valid_user_id(&to) {
        return (err("destinatario invalido"), None);
    }
    if sealed_from.len() > MAX_SEALED_FROM || ciphertext.is_empty() || ciphertext.len() > MAX_CIPHERTEXT {
        return (err("mensagem com tamanho invalido"), None);
    }
    if !crate::limits::allow(crate::limits::Limited::Send, me, cfg.send_rate) {
        stats::inc(&STATS.rate_limited);
        return (err(crate::protocol::RATE_LIMITED), None);
    }
    // Fazer chamadas e pago (0.9.21): a oferta vai com a campainha marcada.
    if wake == Some(crate::protocol::WakeKind::Call) {
        match store.premium_allows(me).await {
            Ok(true) => {}
            Ok(false) => return (err(crate::protocol::PREMIUM_REQUIRED), None),
            Err(e) => return (internal(e), None),
        }
    }
    let recipient = to.clone();
    // Leitura telefone a telefone (latency.rs): so mensagens normais para quem tem a app
    // aberta a espera neste instante (visto antes de o acordar).
    let live = !silent && wake.is_none() && crate::waiters::is_waiting(&recipient);
    match store.enqueue_message(me, to, sealed_from, ciphertext).await {
        Ok(id) => {
            stats::inc(&STATS.sends);
            crate::latency::queued(id, live);
            // Entrega imediata: acorda o destinatario se estiver a espera (WaitMessages).
            crate::waiters::wake(&recipient);
            // App fechada (ninguem a espera): campainha pela Google (FCM), vazia.
            // Recibos, reacoes e presenca (`silent`) nao tocam. Chamadas: campainha marcada.
            if !silent || wake.is_some() {
                crate::push::after_enqueue(store, &recipient, wake).await;
            }
            (ServerMessage::Ack, Some(id))
        }
        Err(StoreError::QueueFull) => {
            stats::inc(&STATS.queue_full);
            (err("fila do destinatario cheia"), None)
        }
        Err(StoreError::RecipientDeleted) => (err(crate::protocol::RECIPIENT_DELETED), None),
        Err(StoreError::SenderNotRegistered) => {
            stats::inc(&STATS.not_registered);
            (err(crate::protocol::NOT_REGISTERED), None)
        }
        Err(StoreError::RecipientUnknown) => {
            stats::inc(&STATS.recipient_unknown);
            (err(crate::protocol::RECIPIENT_UNKNOWN), None)
        }
        Err(e) => (internal(e), None),
    }
}
