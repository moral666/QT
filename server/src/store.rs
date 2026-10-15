//! Estado do servidor/relay, agora persistido em REDIS de verdade (em vez
//! de HashMap em memória) - sobrevive a reinícios do processo, e as filas
//! de mensagens têm TTL automático (o Redis apaga-as sozinho se ninguém as
//! for buscar dentro do prazo, evitando acumulação indefinida de contas
//! abandonadas).
//!
//! Continua "cego" quanto ao conteúdo: os bundles e os `sealed_from`/
//! `ciphertext` são guardados como bytes opacos, exatamente como recebidos.
//!
//! Formato de cada entrada da fila: ver `encode_entry` (id + sealed_from + ciphertext).
//! Escolhido deliberadamente sem serde/JSON para evitar overhead e
//! dependências extra - é só concatenação de bytes com um prefixo de tamanho.

use crate::protocol::UserId;
use redis::AsyncCommands;
use thiserror::Error;

/// Limite de mensagens em fila por utilizador - mitigação simples de DoS
/// (encher a fila de alguém offline com lixo).
const MAX_QUEUED_MESSAGES_PER_USER: usize = 1000;

/// TTL da fila de um utilizador: 30 dias sem ser levantada, o Redis apaga-a
/// sozinha. Consistente com a política de retenção descrita em
/// docs/protocol-spec.md.
const QUEUE_TTL_SECONDS: i64 = 60 * 60 * 24 * 30;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("erro de ligação/comando Redis: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("fila do destinatário cheia")]
    QueueFull,
}

pub struct Store {
    client: redis::Client,
    /// Uma unica ligacao ao Redis, partilhada por todos os pedidos (os clones
    /// sao baratos e multiplexam na mesma ligacao). Criada no primeiro pedido;
    /// se o Redis reiniciar, volta a ligar sozinha.
    shared: tokio::sync::OnceCell<redis::aio::ConnectionManager>,
    /// Prefixo de namespace para as chaves Redis - permite que múltiplas
    /// instâncias (ex.: testes em paralelo) partilhem o mesmo servidor
    /// Redis sem colidirem entre si.
    pub(crate) key_prefix: String,
    /// Anexos no disco (ver `blobs.rs`); `None` = desativados.
    pub(crate) blobs: Option<crate::blobs::Blobs>,
}

impl Store {
    /// `redis_url` tipicamente `"redis://127.0.0.1:6379"`.
    pub fn new(redis_url: &str) -> Result<Self, StoreError> {
        Self::with_prefix(redis_url, "default")
    }

    pub fn with_prefix(redis_url: &str, key_prefix: &str) -> Result<Self, StoreError> {
        let client = redis::Client::open(redis_url)?;
        Ok(Self { client, shared: tokio::sync::OnceCell::new(), key_prefix: key_prefix.to_string(), blobs: None })
    }

    /// Ativa os anexos, guardados em `cfg.dir` (criada com permissoes 0700).
    pub fn enable_blobs(&mut self, cfg: crate::blobs::BlobConfig) -> std::io::Result<()> {
        self.blobs = Some(crate::blobs::Blobs::new(cfg)?);
        Ok(())
    }

    pub fn blobs_enabled(&self) -> bool {
        self.blobs.is_some()
    }

    /// Antes abria uma ligacao TCP nova ao Redis em CADA pedido; com 10 000
    /// ligacoes a pedir mensagens de 3 em 3 s, reutilizar uma so ligacao baixou
    /// o CPU (relay + Redis) em cerca de 58% (ver docs/performance.md).
    pub(crate) async fn connection(&self) -> Result<redis::aio::ConnectionManager, StoreError> {
        let manager = self
            .shared
            .get_or_try_init(|| redis::aio::ConnectionManager::new(self.client.clone()))
            .await?;
        Ok(manager.clone())
    }

    pub async fn register_bundle(&self, user_id: UserId, bundle_bytes: Vec<u8>) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let key = format!("{}:bundle:{}", self.key_prefix, user_id);
        conn.set::<_, _, ()>(key, bundle_bytes).await?;
        Ok(())
    }

    pub async fn get_bundle(&self, user_id: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let mut conn = self.connection().await?;
        let key = format!("{}:bundle:{}", self.key_prefix, user_id);
        let value: Option<Vec<u8>> = conn.get(key).await?;
        Ok(value)
    }

    pub async fn enqueue_message(
        &self,
        to: UserId,
        sealed_from: Vec<u8>,
        ciphertext: Vec<u8>,
    ) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let key = format!("{}:queue:{}", self.key_prefix, to);

        // Duas idas ao Redis em vez de quatro: (tamanho da fila + id novo) e depois
        // (guardar + renovar o TTL), cada par num so pedido (pipeline). Os comandos
        // sao os mesmos de antes; um id gasto com a fila cheia nao faz falta (os ids
        // so tem de ser unicos).
        let (current_len, id): (usize, u64) = redis::pipe()
            .llen(&key)
            .incr(format!("{}:seq", self.key_prefix), 1u64)
            .query_async(&mut conn)
            .await?;
        if current_len >= MAX_QUEUED_MESSAGES_PER_USER {
            return Err(StoreError::QueueFull);
        }

        let entry = encode_entry(id, &sealed_from, &ciphertext);
        // Renova o TTL a cada nova mensagem - a fila só expira 30 dias
        // depois da ÚLTIMA mensagem recebida, não da primeira.
        redis::pipe()
            .rpush(&key, entry)
            .ignore()
            .expire(&key, QUEUE_TTL_SECONDS)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;

        Ok(())
    }

    /// Devolve (SEM apagar) as proximas mensagens da fila, ate `max_bytes`
    /// estimados de payload (para caber num frame Noise). Entradas corrompidas
    /// sao descartadas em vez de provocarem panic.
    pub async fn peek_messages(
        &self,
        user_id: &str,
        max_bytes: usize,
    ) -> Result<Vec<QueuedMessage>, StoreError> {
        let mut conn = self.connection().await?;
        let key = format!("{}:queue:{}", self.key_prefix, user_id);
        let raw_entries: Vec<Vec<u8>> = conn.lrange(&key, 0, 99).await?;
        let mut out = Vec::new();
        let mut budget = 0usize;
        for raw in raw_entries {
            match decode_entry(&raw) {
                Some(m) => {
                    // JSON serializa cada byte como ate 4 caracteres.
                    let est = (m.sealed_from.len() + m.ciphertext.len()) * 4 + 64;
                    if !out.is_empty() && budget + est > max_bytes {
                        break;
                    }
                    budget += est;
                    out.push(m);
                }
                None => {
                    conn.lrem::<_, _, ()>(&key, 1, raw).await?;
                }
            }
        }
        Ok(out)
    }

    /// Apaga da fila as mensagens confirmadas (por id).
    pub async fn ack_messages(&self, user_id: &str, ids: &[u64]) -> Result<usize, StoreError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.connection().await?;
        let key = format!("{}:queue:{}", self.key_prefix, user_id);
        let raw_entries: Vec<Vec<u8>> = conn.lrange(&key, 0, -1).await?;
        let mut removed = 0usize;
        for raw in raw_entries {
            if let Some(m) = decode_entry(&raw) {
                if ids.contains(&m.id) {
                    removed += conn.lrem::<_, _, usize>(&key, 1, raw).await?;
                }
            }
        }
        Ok(removed)
    }
}

#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub id: u64,
    pub sealed_from: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

/// Formato: [8 id BE][4 len(sealed_from) BE][sealed_from][ciphertext]
fn encode_entry(id: u64, sealed_from: &[u8], ciphertext: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + sealed_from.len() + ciphertext.len());
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&(sealed_from.len() as u32).to_be_bytes());
    out.extend_from_slice(sealed_from);
    out.extend_from_slice(ciphertext);
    out
}

fn decode_entry(bytes: &[u8]) -> Option<QueuedMessage> {
    if bytes.len() < 12 {
        return None;
    }
    let id = u64::from_be_bytes(bytes[0..8].try_into().ok()?);
    let len = u32::from_be_bytes(bytes[8..12].try_into().ok()?) as usize;
    let end = 12usize.checked_add(len)?;
    if bytes.len() < end {
        return None;
    }
    Some(QueuedMessage { id, sealed_from: bytes[12..end].to_vec(), ciphertext: bytes[end..].to_vec() })
}
