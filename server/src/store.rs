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

/// TTL do bundle de pre-keys: as apps voltam a publica-lo sempre que abrem
/// (arranque ou desbloqueio), por isso so expira para quem nao abre a app ha 90 dias (ou a
/// desinstalou sem apagar a conta). Antes nao expirava nunca.
pub const BUNDLE_TTL_SECONDS: u64 = 60 * 60 * 24 * 90;

/// TTL do token de notificacoes (FCM): como o bundle, renovado sempre que a app abre.
pub const PUSH_TTL_SECONDS: u64 = BUNDLE_TTL_SECONDS;

/// Quanto tempo o relay se lembra de que uma conta foi apagada (para recusar
/// mensagens para ela, em vez de as guardar 30 dias sem ninguem as ir buscar).
/// So guarda o `user_id` (a chave publica), nada mais.
pub const DELETED_TTL_SECONDS: u64 = 60 * 60 * 24 * 30;

/// Denuncias (cifradas para o operador): apagadas sozinhas ao fim de 30 dias.
pub const REPORT_TTL_SECONDS: u64 = 60 * 60 * 24 * 30;
/// Denuncias por conta e por dia (contra quem quer encher o relay de denuncias).
pub const MAX_REPORTS_PER_DAY: u64 = 10;
/// Denuncias guardadas ao mesmo tempo, no maximo (o operador apaga as tratadas).
pub const MAX_REPORTS_STORED: usize = 5_000;

/// Entradas lidas do Redis por pedido de mensagens: primeiro estas...
const PEEK_FIRST: isize = 16;
/// ...e, se couberem todas na pagina, ate este total (o limite de sempre).
const PEEK_MAX: isize = 100;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("erro de ligação/comando Redis: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("fila do destinatário cheia")]
    QueueFull,
    #[error("o destinatário apagou a conta")]
    RecipientDeleted,
    /// Quem envia nao tem bundle no relay (nunca registou, ou expirou ha mais de
    /// 90 dias). Sem isto, chaves descartaveis (sem prova de trabalho) enchiam o Redis.
    #[error("o remetente nao tem conta registada")]
    SenderNotRegistered,
    /// O destinatario nao tem bundle: o ID nunca existiu (ou expirou). Sem isto,
    /// qualquer conta enchia filas de IDs inventados (1000 mensagens cada, 30 dias).
    #[error("destinatário desconhecido")]
    RecipientUnknown,
    /// A conta ja fez `MAX_REPORTS_PER_DAY` denuncias hoje.
    #[error("limite diario de denuncias")]
    ReportLimit,
    /// Ha `MAX_REPORTS_STORED` denuncias por tratar.
    #[error("demasiadas denuncias guardadas")]
    ReportsFull,
}

fn today() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
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

    fn deleted_key(&self, user_id: &str) -> String {
        format!("{}:deleted:{}", self.key_prefix, user_id)
    }

    /// Guarda (ou renova) o bundle, com `BUNDLE_TTL_SECONDS`. Publicar um bundle
    /// tambem desfaz uma eliminacao anterior (a identidade esta viva).
    pub async fn register_bundle(&self, user_id: UserId, bundle_bytes: Vec<u8>) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let key = format!("{}:bundle:{}", self.key_prefix, user_id);
        // So comandos permitidos pelo ACL do relay (deploy/redis/qt-redis.conf):
        // SET + EXPIRE em vez de SETEX.
        redis::pipe()
            .set(&key, bundle_bytes)
            .ignore()
            .expire(&key, BUNDLE_TTL_SECONDS as i64)
            .ignore()
            .del(self.deleted_key(&user_id))
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
    }

    /// Apaga tudo o que o relay guarda deste utilizador: bundle, fila de mensagens
    /// e contagem de anexos (de hoje e de ontem; os anexos em si nao estao ligados
    /// a quem os enviou e apagam-se sozinhos em 24 horas). Fica so a marca de conta
    /// apagada, durante `DELETED_TTL_SECONDS`.
    pub async fn delete_account(&self, user_id: &str) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        let day = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() / 86_400)
            .unwrap_or(0);
        redis::pipe()
            .del(format!("{p}:bundle:{user_id}"))
            .ignore()
            .del(format!("{p}:otpk:{user_id}"))
            .ignore()
            .del(format!("{p}:queue:{user_id}"))
            .ignore()
            .del(format!("{p}:push:{user_id}"))
            .ignore()
            .del(format!("{p}:blobquota:{user_id}:{day}"))
            .ignore()
            .del(format!("{p}:blobquota:{user_id}:{}", day.saturating_sub(1)))
            .ignore()
            .set(self.deleted_key(user_id), 1u8)
            .ignore()
            .expire(self.deleted_key(user_id), DELETED_TTL_SECONDS as i64)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
    }

    /// Guarda (`Some`) ou apaga (`None`) o token de notificacoes do utilizador.
    pub async fn set_push_token(&self, user_id: &str, token: Option<&str>) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let key = format!("{}:push:{}", self.key_prefix, user_id);
        match token {
            Some(t) => {
                redis::pipe()
                    .set(&key, t)
                    .ignore()
                    .expire(&key, PUSH_TTL_SECONDS as i64)
                    .ignore()
                    .query_async::<_, ()>(&mut conn)
                    .await?
            }
            None => conn.del::<_, ()>(&key).await?,
        }
        Ok(())
    }

    pub async fn get_push_token(&self, user_id: &str) -> Result<Option<String>, StoreError> {
        let mut conn = self.connection().await?;
        Ok(conn.get(format!("{}:push:{}", self.key_prefix, user_id)).await?)
    }

    /// Substitui as one-time prekeys do utilizador (lista no Redis, com o TTL do bundle).
    pub async fn replace_one_time_prekeys(&self, user_id: &str, keys: &[Vec<u8>]) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let key = format!("{}:otpk:{}", self.key_prefix, user_id);
        let mut pipe = redis::pipe();
        pipe.atomic().del(&key).ignore();
        if !keys.is_empty() {
            pipe.rpush(&key, keys).ignore().expire(&key, BUNDLE_TTL_SECONDS as i64).ignore();
        }
        pipe.query_async::<_, ()>(&mut conn).await?;
        Ok(())
    }

    /// Tira (e apaga) uma one-time prekey do utilizador, se houver. LPOP e atomico:
    /// dois pedidos ao mesmo tempo nunca recebem a mesma.
    pub async fn pop_one_time_prekey(&self, user_id: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let mut conn = self.connection().await?;
        Ok(conn.lpop(format!("{}:otpk:{}", self.key_prefix, user_id), None).await?)
    }

    /// Quantas one-time prekeys restam (testes e estatisticas).
    pub async fn count_one_time_prekeys(&self, user_id: &str) -> Result<usize, StoreError> {
        let mut conn = self.connection().await?;
        Ok(conn.llen(format!("{}:otpk:{}", self.key_prefix, user_id)).await?)
    }

    pub async fn get_bundle(&self, user_id: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let mut conn = self.connection().await?;
        let key = format!("{}:bundle:{}", self.key_prefix, user_id);
        let value: Option<Vec<u8>> = conn.get(key).await?;
        Ok(value)
    }

    /// A conta tem bundle publicado (passou a prova de trabalho e abriu a app nos
    /// ultimos 90 dias)? So estas contas podem escrever no relay alem do registo.
    pub async fn is_registered(&self, user_id: &str) -> Result<bool, StoreError> {
        Ok(self.get_bundle(user_id).await?.is_some())
    }

    /// Poe uma mensagem na fila de `to`. So entre contas registadas: `from` (quem
    /// esta autenticado na ligacao) e `to` tem de ter bundle no relay.
    pub async fn enqueue_message(
        &self,
        from: &str,
        to: UserId,
        sealed_from: Vec<u8>,
        ciphertext: Vec<u8>,
    ) -> Result<u64, StoreError> {
        let mut conn = self.connection().await?;
        let key = format!("{}:queue:{}", self.key_prefix, to);

        // Duas idas ao Redis em vez de quatro: (verificacoes + id novo) e depois
        // (guardar + renovar o TTL), cada grupo num so pedido (pipeline). Um id gasto
        // numa mensagem recusada nao faz falta (os ids so tem de ser unicos).
        // GET em vez de EXISTS: so comandos permitidos pelo ACL do relay (o bundle
        // tem ~1,4 KB, lido no mesmo pedido, sem ida extra ao Redis).
        type Checks = (usize, u64, Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>);
        let (current_len, id, deleted, to_bundle, from_bundle): Checks = redis::pipe()
            .llen(&key)
            .incr(format!("{}:seq", self.key_prefix), 1u64)
            .get(self.deleted_key(&to))
            .get(format!("{}:bundle:{}", self.key_prefix, to))
            .get(format!("{}:bundle:{}", self.key_prefix, from))
            .query_async(&mut conn)
            .await?;
        // Por esta ordem: "apagou a conta" continua a ser a resposta para quem apagou
        // (a app mostra um aviso), mesmo que o bundle ja nao exista.
        if deleted.is_some() {
            return Err(StoreError::RecipientDeleted);
        }
        if from_bundle.is_none() {
            return Err(StoreError::SenderNotRegistered);
        }
        if to_bundle.is_none() {
            return Err(StoreError::RecipientUnknown);
        }
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

        // O id (interno da fila) serve so para medir o tempo de entrega (latency.rs).
        Ok(id)
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
        let mut out = Vec::new();
        let mut budget = 0usize;
        // Primeiro so as 16 primeiras entradas: com mensagens grandes, so 1 a 3 cabem
        // numa pagina e ler 100 do Redis era desperdicio. So se couberem todas e que
        // le as seguintes (ate 100, como antes).
        let mut from = 0isize;
        for to in [PEEK_FIRST - 1, PEEK_MAX - 1] {
            let raw_entries: Vec<Vec<u8>> = conn.lrange(&key, from, to).await?;
            let got = raw_entries.len();
            for raw in raw_entries {
                match decode_entry(&raw) {
                    Some(m) => {
                        // JSON serializa cada byte como ate 4 caracteres.
                        let est = (m.sealed_from.len() + m.ciphertext.len()) * 4 + 64;
                        if !out.is_empty() && budget + est > max_bytes {
                            return Ok(out);
                        }
                        budget += est;
                        out.push(m);
                    }
                    None => {
                        conn.lrem::<_, _, ()>(&key, 1, raw).await?;
                    }
                }
            }
            if got < (to - from + 1) as usize {
                break; // nao ha mais entradas
            }
            from = to + 1;
        }
        Ok(out)
    }

    /// Apaga da fila as mensagens confirmadas (por id).
    ///
    /// O cliente confirma o que acabou de ler, que esta no INICIO da fila: le so
    /// essas entradas (antes lia a fila inteira a cada confirmacao, o que ficava
    /// mais lento quanto maior a fila) e apaga-as todas num so pedido ao Redis.
    /// So se alguma nao estiver no inicio e que procura no resto da fila.
    /// LREM apaga a entrada exata: duas confirmacoes ao mesmo tempo (ex.: uma
    /// ligacao antiga e a nova) nunca apagam mensagens por confirmar.
    pub async fn ack_messages(&self, user_id: &str, ids: &[u64]) -> Result<usize, StoreError> {
        Ok(self.ack_messages_ids(user_id, ids).await?.len())
    }

    /// Como `ack_messages`, mas devolve os ids que sairam mesmo da fila (para a
    /// medicao do tempo de entrega nao contar ids repetidos ou alheios).
    pub async fn ack_messages_ids(&self, user_id: &str, ids: &[u64]) -> Result<Vec<u64>, StoreError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.connection().await?;
        let key = format!("{}:queue:{}", self.key_prefix, user_id);
        let mut wanted: std::collections::HashSet<u64> = ids.iter().copied().collect();
        let mut pipe = redis::pipe();
        let mut pending = 0usize;
        let mut order: Vec<u64> = Vec::new();
        let first = wanted.len();
        let head: Vec<Vec<u8>> = conn.lrange(&key, 0, first as isize - 1).await?;
        let scanned = head.len();
        for raw in head {
            if let Some(m) = decode_entry(&raw) {
                if wanted.remove(&m.id) {
                    pipe.lrem(&key, 1, raw);
                    pending += 1;
                    order.push(m.id);
                }
            }
        }
        // A fila tem mais entradas para la das que lemos: procura as que faltam.
        if !wanted.is_empty() && scanned == first {
            let rest: Vec<Vec<u8>> = conn.lrange(&key, scanned as isize, -1).await?;
            for raw in rest {
                if wanted.is_empty() {
                    break;
                }
                if let Some(m) = decode_entry(&raw) {
                    if wanted.remove(&m.id) {
                        pipe.lrem(&key, 1, raw);
                        pending += 1;
                        order.push(m.id);
                    }
                }
            }
        }
        if pending == 0 {
            return Ok(Vec::new());
        }
        let removed: Vec<usize> = pipe.query_async(&mut conn).await?;
        Ok(order.into_iter().zip(removed).filter(|(_, n)| *n > 0).map(|(id, _)| id).collect())
    }
}

// ------------------------------------------------------------ denuncias e banimentos
//
// Chaves (so comandos permitidos pelo ACL do relay):
//   {p}:report:{id}            [8 recebida_em ms BE][denuncia cifrada], TTL 30 dias
//   {p}:reports                lista dos ids (os que expiraram saem ao listar)
//   {p}:report_seq             contador dos ids
//   {p}:reportday:{conta}:{dia} denuncias feitas hoje por esta conta (TTL 2 dias). E o
//                              unico sitio onde aparece quem denuncia, e nao diz qual.
//   {p}:ban:{conta}            conta banida (sem TTL)
//   {p}:bans                   lista das contas banidas
impl Store {
    /// Guarda uma denuncia (bytes opacos). Devolve o id.
    pub async fn submit_report(&self, reporter: &str, sealed: Vec<u8>) -> Result<u64, StoreError> {
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        let day_key = format!("{p}:reportday:{reporter}:{}", today());
        let (count, stored): (u64, usize) = redis::pipe()
            .incr(&day_key, 1u64)
            .expire(&day_key, 2 * 86_400)
            .ignore()
            .llen(format!("{p}:reports"))
            .query_async(&mut conn)
            .await?;
        if count > MAX_REPORTS_PER_DAY {
            return Err(StoreError::ReportLimit);
        }
        if stored >= MAX_REPORTS_STORED {
            return Err(StoreError::ReportsFull);
        }
        let id: u64 = conn.incr(format!("{p}:report_seq"), 1u64).await?;
        let key = format!("{p}:report:{id}");
        let mut value = now_ms().to_be_bytes().to_vec();
        value.extend_from_slice(&sealed);
        redis::pipe()
            .set(&key, value)
            .ignore()
            .expire(&key, REPORT_TTL_SECONDS as i64)
            .ignore()
            .rpush(format!("{p}:reports"), id)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(id)
    }

    /// Denuncias com id maior do que `after`, ate `max_bytes` estimados. `more`: ha mais.
    pub async fn list_reports(
        &self,
        after: u64,
        max_bytes: usize,
    ) -> Result<(Vec<crate::protocol::StoredReport>, bool), StoreError> {
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        let list = format!("{p}:reports");
        let ids: Vec<u64> = conn.lrange(&list, 0, -1).await?;
        let mut out = Vec::new();
        let mut budget = 0usize;
        for id in ids.into_iter().filter(|id| *id > after) {
            let raw: Option<Vec<u8>> = conn.get(format!("{p}:report:{id}")).await?;
            let Some(raw) = raw.filter(|r| r.len() > 8) else {
                // Expirou (30 dias) ou estragada: sai da lista.
                conn.lrem::<_, _, ()>(&list, 0, id).await?;
                continue;
            };
            let est = raw.len() * 4 + 64; // JSON: ate 4 caracteres por byte
            if !out.is_empty() && budget + est > max_bytes {
                return Ok((out, true));
            }
            budget += est;
            out.push(crate::protocol::StoredReport {
                id,
                received_at_ms: u64::from_be_bytes(raw[..8].try_into().unwrap_or_default()),
                sealed: raw[8..].to_vec(),
            });
        }
        Ok((out, false))
    }

    pub async fn delete_report(&self, id: u64) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        redis::pipe()
            .del(format!("{p}:report:{id}"))
            .ignore()
            .lrem(format!("{p}:reports"), 0, id)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
    }

    /// Bane a conta: apaga o bundle, as one-time prekeys, a fila e a campainha, e marca-a.
    pub async fn ban(&self, user_id: &str) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        redis::pipe()
            .del(format!("{p}:bundle:{user_id}"))
            .ignore()
            .del(format!("{p}:otpk:{user_id}"))
            .ignore()
            .del(format!("{p}:queue:{user_id}"))
            .ignore()
            .del(format!("{p}:push:{user_id}"))
            .ignore()
            .set(format!("{p}:ban:{user_id}"), 1u8)
            .ignore()
            .lrem(format!("{p}:bans"), 0, user_id)
            .ignore()
            .rpush(format!("{p}:bans"), user_id)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn unban(&self, user_id: &str) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        redis::pipe()
            .del(format!("{p}:ban:{user_id}"))
            .ignore()
            .lrem(format!("{p}:bans"), 0, user_id)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn is_banned(&self, user_id: &str) -> Result<bool, StoreError> {
        let mut conn = self.connection().await?;
        let v: Option<Vec<u8>> = conn.get(format!("{}:ban:{}", self.key_prefix, user_id)).await?;
        Ok(v.is_some())
    }

    pub async fn list_bans(&self) -> Result<Vec<String>, StoreError> {
        let mut conn = self.connection().await?;
        Ok(conn.lrange(format!("{}:bans", self.key_prefix), 0, -1).await?)
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
