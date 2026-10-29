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
    /// Emissor das fichas do video (0006): chaves do mes, da semente. `None` = desligado.
    pub fichas: Option<Arc<crate::fichas::IssuerKeys>>,
    /// Fichas por conta e por mes (1 ficha = 1 minuto de video pelo relay): quota configuravel.
    pub fichas_per_month: u32,
    /// Pedidos de fichas por conta (`IssueFichas`).
    pub fichas_rate: crate::limits::Rate,
    /// Federacao de relays (`federation.rs`): `None` = relay sozinho.
    pub federacao: Option<Arc<crate::federation::Federacao>>,
    /// Aceita contas novas (`RELAY_NOVAS_CONTAS=0` desliga: relay cheio, decidido pelo operador).
    /// As contas que ja tem continuam a renovar o registo.
    pub novas_contas: bool,
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
            fichas: None,
            fichas_per_month: qt_core::fichas::DEFAULT_PER_MONTH,
            // Um lote por mes chega; a folga e para quotas maiores (varios lotes) e repeticoes.
            fichas_rate: crate::limits::Rate { burst: 6, per_minute: 2 },
            federacao: None,
            novas_contas: true,
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
            match store.is_banned(me).await {
                Ok(false) => {}
                Ok(true) => return err(crate::protocol::BANNED),
                Err(e) => return internal(e),
            }
            // Conta nova (ou que expirou): so com prova de trabalho valida.
            let nova = match store.get_bundle(me).await {
                Ok(b) => b.is_none(),
                Err(e) => return internal(e),
            };
            if nova
                && cfg.require_pow
                && !pow.is_some_and(|n| qt_core::pow::pow_verify(&signing, n, qt_core::pow::POW_BITS))
            {
                stats::inc(&STATS.pow_rejected);
                return err(crate::protocol::POW_REQUIRED);
            }
            // Relay fechado a contas novas (operador): se a conta ja existir noutro relay, diz
            // qual (sem reservar nada, para o registo la nao encontrar uma reserva daqui).
            if nova && !cfg.novas_contas {
                if let Some(fed) = &cfg.federacao {
                    if let crate::federation::Onde::Par(n) = fed.onde_esta(me).await {
                        return err(&crate::protocol::elsewhere(Some(&n)));
                    }
                }
                return err(crate::protocol::NO_NEW_ACCOUNTS);
            }
            // Federacao: uma conta so existe num relay. So contas novas perguntam aos outros;
            // renovar o bundle (cada vez que a app abre) nao gasta nenhum pedido.
            let mut por_verificar = false;
            let fed = cfg.federacao.as_ref().filter(|_| nova);
            if let Some(fed) = fed {
                match fed.reservar(me).await {
                    crate::federation::Reserva::Livre { por_verificar: pv } => por_verificar = pv,
                    crate::federation::Reserva::NoutroRelay(n) => return err(&crate::protocol::elsewhere(n.as_deref())),
                    crate::federation::Reserva::Banida => return err(crate::protocol::BANNED),
                }
            }
            let registo = store.register_bundle(user_id, bundle_bytes).await;
            if registo.is_ok() && por_verificar {
                // Algum relay nao respondeu: fica para verificar quando ele voltar (A1).
                let _ = store.fed_marcar_por_verificar(me).await;
            }
            if let Some(fed) = fed {
                fed.libertar(me);
            }
            match registo {
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
                    // Federacao (fase 2): a conta pode estar noutro relay.
                    if let Some(fed) = &cfg.federacao {
                        if let Some(r) = fed.pedir_bundle(&user_id).await {
                            stats::inc(&STATS.fed_lookups);
                            stats::inc(&STATS.bundles_fetched);
                            return r;
                        }
                    }
                    stats::inc(&STATS.bundles_not_found);
                    ServerMessage::PreKeyBundleNotFound
                }
                Err(e) => internal(e),
            }
        }
        ClientMessage::SendMessage { to, sealed_from, ciphertext, silent, wake } => {
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
            // Fazer chamadas e pago (0.9.21): a oferta vai com a campainha marcada.
            if wake == Some(crate::protocol::WakeKind::Call) {
                match store.premium_allows(me).await {
                    Ok(true) => {}
                    Ok(false) => return err(crate::protocol::PREMIUM_REQUIRED),
                    Err(e) => return internal(e),
                }
            }
            let recipient = to.clone();
            // Copias so para a federacao (a fila local consome as originais).
            let fed_copy = cfg.federacao.as_ref().map(|_| (sealed_from.clone(), ciphertext.clone()));
            let local = store.enqueue_message(me, to, sealed_from, ciphertext).await;
            // Federacao (fase 2): destinatario que nao esta neste relay (ou que apagou a conta
            // AQUI e pode existir noutro). So para quem esta registado neste relay.
            if let (Some(fed), Some((sf, ct)), Err(StoreError::RecipientUnknown | StoreError::RecipientDeleted)) =
                (&cfg.federacao, &fed_copy, &local)
            {
                if matches!(local, Err(StoreError::RecipientUnknown)) || matches!(store.is_registered(me).await, Ok(true)) {
                    match fed.entregar(&recipient, sf, ct, silent, wake).await {
                        crate::federation::Entrega::Entregue => {
                            stats::inc(&STATS.sends);
                            stats::inc(&STATS.fed_out);
                            return ServerMessage::Ack;
                        }
                        crate::federation::Entrega::Recusado(motivo) => return err(&motivo),
                        crate::federation::Entrega::NaoEsta => {}
                    }
                }
            }
            match local {
                Ok(id) => {
                    stats::inc(&STATS.sends);
                    // Tempo de entrega: so conta se o destinatario estava ligado (app
                    // aberta). Com a app fechada mediria quanto tempo a pessoa demorou
                    // a abri-la, nao quanto o servico demorou a entregar.
                    if crate::waiters::is_online(&recipient) {
                        crate::latency::queued(id);
                    }
                    // Entrega imediata: acorda o destinatario se estiver a espera (WaitMessages).
                    crate::waiters::wake(&recipient);
                    // App fechada (ninguem a espera): campainha pela Google (FCM), vazia.
                    // Recibos, reacoes e presenca (`silent`) nao tocam. Chamadas: campainha marcada.
                    if !silent || wake.is_some() {
                        crate::push::after_enqueue(store, &recipient, wake).await;
                    }
                    ServerMessage::Ack
                }
                Err(StoreError::QueueFull) => {
                    stats::inc(&STATS.queue_full);
                    err("fila do destinatario cheia")
                }
                Err(StoreError::RecipientDeleted) => err(crate::protocol::RECIPIENT_DELETED),
                Err(StoreError::SenderNotRegistered) => {
                    stats::inc(&STATS.not_registered);
                    err(crate::protocol::NOT_REGISTERED)
                }
                Err(StoreError::RecipientUnknown) => {
                    stats::inc(&STATS.recipient_unknown);
                    err(crate::protocol::RECIPIENT_UNKNOWN)
                }
                Err(e) => internal(e),
            }
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
        // Federacao (fase 2): um anexo que nao esta aqui pode estar no relay de quem o enviou.
        ClientMessage::BlobInfo { blob_id } => match store.blob_info(&blob_id).await {
            Ok(s) => ServerMessage::BlobStatus { size: s.size, received: s.received, complete: s.complete },
            Err(BlobError::NotFound) if cfg.federacao.is_some() => {
                let fed = cfg.federacao.as_ref().unwrap();
                fed.info_anexo(&blob_id).await.unwrap_or(ServerMessage::BlobNotFound)
            }
            Err(e) => blob_err(e),
        },
        ClientMessage::BlobGet { blob_id, offset } => match store.blob_get(&blob_id, offset).await {
            Ok((size, data)) => {
                if offset == 0 {
                    stats::inc(&STATS.blob_downloads);
                }
                ServerMessage::BlobData { offset, size, data: b64().encode(data) }
            }
            Err(BlobError::NotFound) if cfg.federacao.is_some() => {
                let fed = cfg.federacao.as_ref().unwrap();
                match fed.ler_anexo(&blob_id, offset).await {
                    Some(r) => {
                        if offset == 0 {
                            stats::inc(&STATS.blob_downloads);
                            stats::inc(&STATS.fed_lookups);
                        }
                        r
                    }
                    None => ServerMessage::BlobNotFound,
                }
            }
            Err(e) => blob_err(e),
        },
        ClientMessage::BlobDelete { blob_id } => {
            if let Some(fed) = &cfg.federacao {
                if matches!(store.blob_info(&blob_id).await, Err(BlobError::NotFound)) {
                    fed.apagar_anexo(&blob_id).await;
                    return ServerMessage::Ack;
                }
            }
            match store.blob_delete(&blob_id).await {
                Ok(()) => ServerMessage::Ack,
                Err(e) => blob_err(e),
            }
        }
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
        | ClientMessage::AdminSetFichasQuota { .. }
        | ClientMessage::AdminOnline
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
            if let Some(r) = admin_noutro_relay(store, cfg, &user_id, crate::federation::AdminFed::Banir { id: user_id.clone() }).await {
                return r;
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
            // O banimento pode estar em qualquer relay: levanta-se em todos.
            if let Some(fed) = &cfg.federacao {
                fed.admin_todos(crate::federation::AdminFed::Desbanir { id: user_id.clone() }).await;
            }
            match store.unban(&user_id).await {
                Ok(()) => ServerMessage::Ack,
                Err(e) => internal(e),
            }
        }
        ClientMessage::AdminOnline => {
            let mut users = crate::limits::UserSlot::users_online(Some(me)) as u64;
            // Federacao: a soma de todos os relays que responderem.
            if let Some(fed) = &cfg.federacao {
                for r in fed.admin_todos(crate::federation::AdminFed::Online).await {
                    if let crate::federation::Resposta::Contagem { n } = r {
                        users += n;
                    }
                }
            }
            ServerMessage::Online { users }
        }
        ClientMessage::AdminListBans => match store.list_bans().await {
            Ok(mut user_ids) => {
                if let Some(fed) = &cfg.federacao {
                    for r in fed.admin_todos(crate::federation::AdminFed::ListarBanidas).await {
                        if let crate::federation::Resposta::ListaBanidas { ids } = r {
                            user_ids.extend(ids.into_iter().filter(|i| is_valid_user_id(i)));
                        }
                    }
                    user_ids.sort();
                    user_ids.dedup();
                }
                ServerMessage::Bans { user_ids }
            }
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
                Ok(None) => {
                    // Federacao (fase 2): a copia pode estar no relay da conta.
                    if let Some(fed) = &cfg.federacao {
                        if let Some(r) = fed.ler_copia(&lookup, part).await {
                            if part == 0 {
                                stats::inc(&STATS.backups_restored);
                                stats::inc(&STATS.fed_lookups);
                            }
                            return r;
                        }
                    }
                    ServerMessage::BackupNotFound
                }
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
                // O bloqueio vale para todos os relays da federacao.
                if let Some(fed) = &cfg.federacao {
                    fed.admin_todos(crate::federation::AdminFed::BloqueioPremium { on }).await;
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
            let a = crate::federation::AdminFed::Premium { id: user_id.clone(), until_ms };
            if let Some(r) = admin_noutro_relay(store, cfg, &user_id, a).await {
                return r;
            }
            match store.set_premium(&user_id, until_ms).await {
                Ok(()) => ServerMessage::Ack,
                Err(e) => internal(e),
            }
        }
        ClientMessage::AdminListPremium => match store.list_premium().await {
            Ok(mut entries) => {
                if let Some(fed) = &cfg.federacao {
                    for r in fed.admin_todos(crate::federation::AdminFed::ListarPremium).await {
                        if let crate::federation::Resposta::ListaPremium { entries: e } = r {
                            entries.extend(e.into_iter().filter(|x| is_valid_user_id(&x.user_id)));
                        }
                    }
                    entries.sort_by(|a, b| a.user_id.cmp(&b.user_id));
                    entries.dedup_by(|a, b| a.user_id == b.user_id);
                }
                ServerMessage::PremiumList { entries }
            }
            Err(e) => internal(e),
        },
        ClientMessage::IssueFichas { epoch, blinded } => issue_fichas(store, me, epoch, &blinded, cfg).await,
        ClientMessage::AdminSetFichasQuota { user_id, per_month } => {
            if !is_valid_user_id(&user_id) {
                return err("user_id invalido");
            }
            let a = crate::federation::AdminFed::QuotaFichas { id: user_id.clone(), per_month };
            if let Some(r) = admin_noutro_relay(store, cfg, &user_id, a).await {
                return r;
            }
            match store.set_fichas_quota(&user_id, per_month).await {
                Ok(()) => ServerMessage::Ack,
                Err(e) => internal(e),
            }
        }
    }
}

/// Federacao (fase 3): pedido do operador para uma conta que vive noutro relay. `None`: a conta
/// esta aqui (ou em relay nenhum) e o pedido faz-se localmente, como antes.
async fn admin_noutro_relay(
    store: &Store,
    cfg: &ServerConfig,
    user_id: &str,
    a: crate::federation::AdminFed,
) -> Option<ServerMessage> {
    let fed = cfg.federacao.as_ref()?;
    if !matches!(store.is_registered(user_id).await, Ok(false)) {
        return None;
    }
    match fed.admin_conta(user_id, a).await? {
        true => Some(ServerMessage::Ack),
        false => Some(err(crate::protocol::FED_UNAVAILABLE)),
    }
}

/// Emissao das fichas do video (0006). So conta quantas deu a conta neste mes: nao guarda nem
/// regista o pedido, a resposta ou a hora (os elementos sao cegos: nao dizem que fichas sao).
async fn issue_fichas(store: &Store, me: &str, epoch: u32, blinded: &str, cfg: &ServerConfig) -> ServerMessage {
    use qt_core::fichas::{current_epoch, ELEMENT_LEN, MAX_BATCH};
    let Some(keys) = cfg.fichas.as_ref() else { return err(crate::protocol::FICHAS_OFF) };
    // base64 de ate MAX_BATCH elementos.
    if blinded.len() > (MAX_BATCH * ELEMENT_LEN).div_ceil(3) * 4 {
        return err("pedido de fichas demasiado grande");
    }
    if !crate::limits::allow(crate::limits::Limited::Fichas, me, cfg.fichas_rate) {
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
    let quota = match store.fichas_quota(me).await {
        Ok(q) => q.unwrap_or(cfg.fichas_per_month),
        Err(e) => return internal(e),
    };
    let now_epoch = current_epoch();
    let Some(issuer) = keys.for_epoch(now_epoch) else {
        stats::inc(&STATS.internal_errors);
        return err("erro interno");
    };
    let public_key = b64().encode(issuer.public_key());
    let reply = |granted: u32, issued: u32, evaluated: &[u8], proof: &[u8]| ServerMessage::Fichas {
        epoch: now_epoch,
        public_key: public_key.clone(),
        evaluated: b64().encode(evaluated),
        proof: b64().encode(proof),
        granted,
        remaining: quota.saturating_sub(issued),
        per_month: quota,
    };
    if epoch != now_epoch {
        // O telemovel esta noutro mes (relogio): diz qual e, sem dar nada.
        return match store.fichas_issued(me, now_epoch).await {
            Ok(issued) => reply(0, issued, &[], &[]),
            Err(e) => internal(e),
        };
    }
    let Ok(raw) = b64().decode(blinded) else { return err("pedido de fichas invalido") };
    if raw.is_empty() || !raw.len().is_multiple_of(ELEMENT_LEN) || raw.len() / ELEMENT_LEN > MAX_BATCH {
        return err("pedido de fichas invalido");
    }
    let n = (raw.len() / ELEMENT_LEN) as u32;
    let (granted, issued) = match store.fichas_take(me, now_epoch, n, quota).await {
        Ok(x) => x,
        Err(e) => return internal(e),
    };
    if granted == 0 {
        return reply(0, issued, &[], &[]);
    }
    // ~1 ms por ficha num servidor pequeno: fora das threads que tratam as ligacoes.
    let part = raw[..granted as usize * ELEMENT_LEN].to_vec();
    let iss = issuer.clone();
    match tokio::task::spawn_blocking(move || iss.issue(&part)).await {
        Ok(Ok((evaluated, proof))) => {
            stats::add(&STATS.fichas_issued, granted as u64);
            reply(granted, issued, &evaluated, &proof)
        }
        _ => {
            // Elementos invalidos: nada foi dado, a reserva volta.
            let _ = store.fichas_refund(me, now_epoch, granted).await;
            err("pedido de fichas invalido")
        }
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
