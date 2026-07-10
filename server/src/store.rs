//! Estado do servidor/relay, agora persistido em REDIS de verdade (em vez
//! de HashMap em memória) - sobrevive a reinícios do processo, e as filas
//! de mensagens têm TTL automático (o Redis apaga-as sozinho se ninguém as
//! for buscar dentro do prazo, evitando acumulação indefinida de contas
//! abandonadas).
//!
//! Continua "cego" quanto ao conteúdo: os bundles e os `sealed_from`/
//! `ciphertext` são guardados como bytes opacos, exatamente como recebidos.
//!
//! Formato de codificação de cada entrada da fila (uma mensagem):
//! [4 bytes u32 BE: tamanho de sealed_from][sealed_from][ciphertext]
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
    /// Prefixo de namespace para as chaves Redis - permite que múltiplas
    /// instâncias (ex.: testes em paralelo) partilhem o mesmo servidor
    /// Redis sem colidirem entre si.
    key_prefix: String,
}

impl Store {
    /// `redis_url` tipicamente `"redis://127.0.0.1:6379"`.
    pub fn new(redis_url: &str) -> Result<Self, StoreError> {
        Self::with_prefix(redis_url, "default")
    }

    pub fn with_prefix(redis_url: &str, key_prefix: &str) -> Result<Self, StoreError> {
        let client = redis::Client::open(redis_url)?;
        Ok(Self { client, key_prefix: key_prefix.to_string() })
    }

    async fn connection(&self) -> Result<redis::aio::MultiplexedConnection, StoreError> {
        Ok(self.client.get_multiplexed_async_connection().await?)
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

        let current_len: usize = conn.llen(&key).await?;
        if current_len >= MAX_QUEUED_MESSAGES_PER_USER {
            return Err(StoreError::QueueFull);
        }

        let entry = encode_entry(&sealed_from, &ciphertext);
        conn.rpush::<_, _, ()>(&key, entry).await?;
        // Renova o TTL a cada nova mensagem - a fila só expira 30 dias
        // depois da ÚLTIMA mensagem recebida, não da primeira.
        conn.expire::<_, ()>(&key, QUEUE_TTL_SECONDS).await?;

        Ok(())
    }

    /// Retira e devolve todas as mensagens em fila para este utilizador
    /// (drena a fila - operação atómica via transação Redis, para não
    /// perder mensagens que cheguem exatamente durante a leitura).
    pub async fn drain_messages(&self, user_id: &str) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError> {
        let mut conn = self.connection().await?;
        let key = format!("{}:queue:{}", self.key_prefix, user_id);

        let (raw_entries, _deleted): (Vec<Vec<u8>>, i64) = redis::pipe()
            .atomic()
            .lrange(&key, 0, -1)
            .del(&key)
            .query_async(&mut conn)
            .await?;

        Ok(raw_entries.iter().map(|e| decode_entry(e)).collect())
    }
}

fn encode_entry(sealed_from: &[u8], ciphertext: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + sealed_from.len() + ciphertext.len());
    out.extend_from_slice(&(sealed_from.len() as u32).to_be_bytes());
    out.extend_from_slice(sealed_from);
    out.extend_from_slice(ciphertext);
    out
}

fn decode_entry(bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let len = u32::from_be_bytes(bytes[0..4].try_into().expect("entrada corrompida na fila")) as usize;
    let sealed_from = bytes[4..4 + len].to_vec();
    let ciphertext = bytes[4 + len..].to_vec();
    (sealed_from, ciphertext)
}
