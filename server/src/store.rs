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

/// Comunidade (0.9.52): a lista (indice e fotos) fica 8 dias: a atualizacao e semanal e um dia
/// de folga evita que a lista desapareca se a atualizacao se atrasar umas horas.
pub const COMMUNITY_TTL_SECONDS: u64 = 60 * 60 * 24 * 8;
/// Envios da Comunidade (fotos seladas para o operador): 30 dias, como as denuncias.
pub const COMMUNITY_SUB_TTL_SECONDS: u64 = 60 * 60 * 24 * 30;
/// Envios por conta e por dia (a app envia so quando a foto ou a medalha muda).
pub const MAX_COMMUNITY_SUBMITS_PER_DAY: u64 = 3;
/// Envios guardados ao mesmo tempo, no maximo (o operador apaga os tratados).
pub const MAX_COMMUNITY_STORED: usize = 2_000;

/// Ids das mensagens reservados de uma vez (`INCRBY seq`): um pedido ao Redis por cada
/// `ID_BLOCK` mensagens em vez de um `INCR` por mensagem. Os ids so tem de ser unicos (as
/// apps so os usam para confirmar); depois de um reinicio fica um buraco, nada mais.
const ID_BLOCK: u64 = 1000;

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
    /// A conta ja fez `MAX_COMMUNITY_SUBMITS_PER_DAY` envios da Comunidade hoje.
    #[error("limite diario de envios da comunidade")]
    CommunityLimit,
    /// Ha `MAX_COMMUNITY_STORED` envios por tratar.
    #[error("demasiados envios da comunidade guardados")]
    CommunityFull,
    /// A copia de recuperacao pertence a outra conta.
    #[error("copia de outra conta")]
    BackupNotOwner,
    /// Faltam partes da copia (a ultima chegou antes das outras).
    #[error("copia incompleta")]
    BackupIncomplete,
}

/// Copia de recuperacao: 90 dias, renovados quando a app abre (`renew_backup`).
pub const BACKUP_TTL_SECONDS: i64 = 60 * 60 * 24 * 90;
/// Partes de uma copia a meio do envio: 1 hora.
const BACKUP_STAGE_TTL_SECONDS: i64 = 60 * 60;

fn today() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

pub(crate) fn now_ms() -> u64 {
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
    /// Bloco de ids reservado no Redis: (proximo, fim exclusivo). Ver `ID_BLOCK`.
    ids: tokio::sync::Mutex<(u64, u64)>,
}

impl Store {
    /// `redis_url` tipicamente `"redis://127.0.0.1:6379"`.
    pub fn new(redis_url: &str) -> Result<Self, StoreError> {
        Self::with_prefix(redis_url, "default")
    }

    pub fn with_prefix(redis_url: &str, key_prefix: &str) -> Result<Self, StoreError> {
        let client = redis::Client::open(redis_url)?;
        Ok(Self {
            client,
            shared: tokio::sync::OnceCell::new(),
            key_prefix: key_prefix.to_string(),
            blobs: None,
            ids: tokio::sync::Mutex::new((0, 0)),
        })
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

    fn bundle_key(&self, user_id: &str) -> String {
        format!("{}:bundle:{}", self.key_prefix, user_id)
    }

    /// Id novo para uma mensagem: do bloco reservado; so quando o bloco acaba vai ao
    /// Redis (`INCRBY seq ID_BLOCK`). Os ids continuam a crescer: um bloco novo vem
    /// sempre acima dos anteriores, tambem depois de reiniciar.
    async fn next_id(&self, conn: &mut redis::aio::ConnectionManager) -> Result<u64, StoreError> {
        let mut block = self.ids.lock().await;
        if block.0 >= block.1 {
            let top: u64 = conn.incr(format!("{}:seq", self.key_prefix), ID_BLOCK).await?;
            *block = (top + 1 - ID_BLOCK, top + 1);
        }
        let id = block.0;
        block.0 += 1;
        Ok(id)
    }

    /// Estado da conta ao autenticar, num so pedido: banida? registada (tem bundle)?
    /// `EXISTS` em vez de `GET`: nao le o bundle (~1,4 KB) so para saber se existe.
    pub async fn auth_state(&self, user_id: &str) -> Result<(bool, bool), StoreError> {
        let mut conn = self.connection().await?;
        let (ban, registered): (Option<Vec<u8>>, bool) = redis::pipe()
            .get(format!("{}:ban:{}", self.key_prefix, user_id))
            .exists(self.bundle_key(user_id))
            .query_async(&mut conn)
            .await?;
        Ok((ban.is_some(), registered))
    }

    /// Registo completo (`RegisterPreKeyBundle`) numa so transacao (MULTI/EXEC): bundle
    /// com TTL, desfaz uma eliminacao anterior, troca as one-time prekeys (se vierem) e le
    /// a ligacao a copia de recuperacao (`bkof`), para a renovar a seguir so se existir.
    /// Devolve o `lookup` da copia, se houver.
    pub async fn register_full(
        &self,
        user_id: &str,
        bundle_bytes: Vec<u8>,
        one_time_pre_keys: Option<&[Vec<u8>]>,
    ) -> Result<Option<String>, StoreError> {
        let mut conn = self.connection().await?;
        let key = self.bundle_key(user_id);
        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.set(&key, bundle_bytes).ignore();
        pipe.expire(&key, BUNDLE_TTL_SECONDS as i64).ignore();
        pipe.del(self.deleted_key(user_id)).ignore();
        if let Some(keys) = one_time_pre_keys {
            let otpk = format!("{}:otpk:{}", self.key_prefix, user_id);
            pipe.del(&otpk).ignore();
            if !keys.is_empty() {
                pipe.rpush(&otpk, keys).ignore().expire(&otpk, BUNDLE_TTL_SECONDS as i64).ignore();
            }
        }
        pipe.get(format!("{}:bkof:{user_id}", self.key_prefix));
        let (lookup,): (Option<String>,) = pipe.query_async(&mut conn).await?;
        Ok(lookup)
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

    /// Chave "de quem e este token" (o token so aparece como SHA-256).
    fn push_owner_key(&self, token: &str) -> String {
        use sha2::{Digest, Sha256};
        let h = Sha256::digest(token.as_bytes());
        let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
        format!("{}:pushowner:{hex}", self.key_prefix)
    }

    /// Guarda (`Some`) ou apaga (`None`) o token de notificacoes do utilizador.
    ///
    /// Um telemovel so acorda por uma conta (0.9.23): se o mesmo token estava noutra conta
    /// (ex.: "Esqueci-me do codigo" apaga a conta so no telemovel e cria outra), essa conta
    /// deixa de o ter. Antes o telemovel continuava a tocar por chamadas para a conta antiga.
    pub async fn set_push_token(&self, user_id: &str, token: Option<&str>) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        let key = format!("{p}:push:{user_id}");
        match token {
            Some(t) => {
                let owner_key = self.push_owner_key(t);
                let mut pipe = redis::pipe();
                let prev: Option<String> = conn.get(&owner_key).await?;
                if let Some(prev) = prev.filter(|u| u != user_id) {
                    // So se essa conta ainda tiver este token (pode ja estar noutro telemovel).
                    let prev_key = format!("{p}:push:{prev}");
                    let cur: Option<String> = conn.get(&prev_key).await?;
                    if cur.as_deref() == Some(t) {
                        pipe.del(&prev_key).ignore();
                    }
                }
                pipe.set(&key, t)
                    .ignore()
                    .expire(&key, PUSH_TTL_SECONDS as i64)
                    .ignore()
                    .set(&owner_key, user_id)
                    .ignore()
                    .expire(&owner_key, PUSH_TTL_SECONDS as i64)
                    .ignore()
                    .query_async::<_, ()>(&mut conn)
                    .await?
            }
            None => {
                let cur: Option<String> = conn.get(&key).await?;
                let mut pipe = redis::pipe();
                if let Some(cur) = cur {
                    let owner_key = self.push_owner_key(&cur);
                    let owner: Option<String> = conn.get(&owner_key).await?;
                    if owner.as_deref() == Some(user_id) {
                        pipe.del(&owner_key).ignore();
                    }
                }
                pipe.del(&key).ignore().query_async::<_, ()>(&mut conn).await?
            }
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
    /// `EXISTS` em vez de ler o bundle (~1,4 KB) so para saber se existe.
    pub async fn is_registered(&self, user_id: &str) -> Result<bool, StoreError> {
        let mut conn = self.connection().await?;
        Ok(conn.exists(self.bundle_key(user_id)).await?)
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
        self.enqueue(Sender::Check(from), to, sealed_from, ciphertext).await
    }

    /// Mensagem vinda de outro relay da federacao (`federation.rs`): quem envia esta registado
    /// LA (o outro relay ja verificou e aplicou os limites dele); aqui so o destinatario conta.
    pub async fn enqueue_from_peer(&self, to: UserId, sealed_from: Vec<u8>, ciphertext: Vec<u8>) -> Result<u64, StoreError> {
        self.enqueue(Sender::Peer, to, sealed_from, ciphertext).await
    }

    /// Poe a mensagem na fila numa so ida ao Redis: as verificacoes (conta apagada,
    /// destinatario e remetente registados, fila cheia) vao no MESMO pedido que a escrita
    /// (RPUSH + EXPIRE), e se alguma falhar a entrada e retirada a seguir (LREM). Antes eram
    /// dois pedidos seguidos (verificar, depois escrever) e um `INCR` por mensagem.
    ///
    /// Escrever antes de verificar e seguro: uma conta apagada ou desconhecida nao tem
    /// ninguem a ler a fila, e numa fila cheia a entrada fica depois da posicao 1000, onde a
    /// entrega (que le no maximo 100) nao chega. Se a fila nao existia, o LREM deixa-a vazia e
    /// o Redis apaga-a: nao fica nada. Se ja existia (bundle expirado, conta levada para outro
    /// relay pela federacao, fila cheia), o desfazer repoe o prazo que ela tinha (`TTL` lido no
    /// mesmo pedido), para uma recusa nunca prolongar a vida de uma fila velha.
    /// So comandos do ACL do relay: LLEN, EXISTS, TTL, RPUSH, EXPIRE, LREM.
    pub async fn enqueue(
        &self,
        from: Sender<'_>,
        to: UserId,
        sealed_from: Vec<u8>,
        ciphertext: Vec<u8>,
    ) -> Result<u64, StoreError> {
        let mut conn = self.connection().await?;
        let key = format!("{}:queue:{}", self.key_prefix, to);
        let id = self.next_id(&mut conn).await?;
        let entry = encode_entry(id, &sealed_from, &ciphertext);
        drop((sealed_from, ciphertext));

        let mut pipe = redis::pipe();
        pipe.llen(&key).exists(self.deleted_key(&to)).exists(self.bundle_key(&to)).ttl(&key);
        if let Sender::Check(f) = from {
            pipe.exists(self.bundle_key(f));
        }
        pipe.rpush(&key, &entry).expire(&key, QUEUE_TTL_SECONDS).ignore();
        let v: Vec<redis::Value> = pipe.query_async(&mut conn).await?;
        let at = |i: usize| v.get(i).ok_or_else(|| redis::RedisError::from((redis::ErrorKind::TypeError, "resposta curta")));
        let len_before: usize = redis::from_redis_value(at(0)?)?;
        let deleted: bool = redis::from_redis_value(at(1)?)?;
        let to_registered: bool = redis::from_redis_value(at(2)?)?;
        let ttl_before: i64 = redis::from_redis_value(at(3)?)?;
        let (from_registered, len_after): (Option<bool>, usize) = match from {
            Sender::Check(_) => (Some(redis::from_redis_value(at(4)?)?), redis::from_redis_value(at(5)?)?),
            _ => (None, redis::from_redis_value(at(4)?)?),
        };

        // Por esta ordem: "apagou a conta" continua a ser a resposta para quem apagou
        // (a app mostra um aviso), mesmo que o bundle ja nao exista.
        let refused = if deleted {
            Some(StoreError::RecipientDeleted)
        } else if from_registered == Some(false) {
            Some(StoreError::SenderNotRegistered)
        } else if !to_registered {
            Some(StoreError::RecipientUnknown)
        } else if len_before >= MAX_QUEUED_MESSAGES_PER_USER {
            Some(StoreError::QueueFull)
        } else {
            None
        };
        if let Some(e) = refused {
            let mut undo = redis::pipe();
            undo.lrem(&key, 1, &entry).ignore();
            if len_after > 1 && ttl_before > 0 {
                undo.expire(&key, ttl_before).ignore();
            }
            undo.query_async::<_, ()>(&mut conn).await?;
            return Err(e);
        }
        // O id (interno da fila) serve so para confirmar e para medir o tempo de entrega
        // (latency.rs).
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
        Ok(self.peek_messages_raw(user_id, max_bytes).await?.into_iter().map(|(m, _)| m).collect())
    }

    /// Como `peek_messages`, com cada entrada tambem em bruto (como esta no Redis): a
    /// ligacao guarda-as para a confirmacao apagar sem voltar a ler a fila (`ack_known`).
    pub async fn peek_messages_raw(
        &self,
        user_id: &str,
        max_bytes: usize,
    ) -> Result<Vec<(QueuedMessage, Vec<u8>)>, StoreError> {
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
                        out.push((m, raw));
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

    /// Confirmacao com as entradas que esta ligacao acabou de entregar (`known`, de
    /// `peek_messages_raw`): LREM pelo valor exato, num so pedido, sem ler a fila antes.
    /// Os ids que nao estiverem em `known` seguem o caminho de sempre (`ack_messages_ids`).
    /// As entradas confirmadas saem de `known`. Como no LREM de sempre, duas ligacoes da
    /// mesma conta a confirmar o mesmo id so o tiram uma vez.
    pub async fn ack_known(
        &self,
        user_id: &str,
        ids: &[u64],
        known: &mut Vec<(u64, Vec<u8>)>,
    ) -> Result<Vec<u64>, StoreError> {
        let mut seen = std::collections::HashSet::new();
        let (mut cached, mut rest) = (Vec::new(), Vec::new());
        for &id in ids {
            if !seen.insert(id) {
                continue;
            }
            match known.iter().position(|(k, _)| *k == id) {
                Some(i) => cached.push(known.swap_remove(i)),
                None => rest.push(id),
            }
        }
        if known.is_empty() {
            *known = Vec::new();
        }
        let mut out = Vec::new();
        if !cached.is_empty() {
            let mut conn = self.connection().await?;
            let key = format!("{}:queue:{}", self.key_prefix, user_id);
            let mut pipe = redis::pipe();
            for (_, raw) in &cached {
                pipe.lrem(&key, 1, raw);
            }
            let removed: Vec<usize> = pipe.query_async(&mut conn).await?;
            out.extend(cached.iter().zip(removed).filter(|(_, n)| *n > 0).map(|((id, _), _)| *id));
        }
        if !rest.is_empty() {
            out.extend(self.ack_messages_ids(user_id, &rest).await?);
        }
        Ok(out)
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

// ------------------------------------------------------------ comunidade (0.9.52)
//
// Chaves (so comandos que o ACL do relay ja permitia):
//   {p}:csub:{id}               [8 recebido_em ms BE][envio selado], TTL 30 dias
//   {p}:csubs                   lista dos ids (os que expiraram saem ao listar)
//   {p}:csub_seq                contador dos ids
//   {p}:csubday:{conta}:{dia}   envios de hoje desta conta (TTL 2 dias). Como nas denuncias,
//                               e o unico sitio com a conta, e nao diz qual envio e dela.
//   {p}:cver                    numero da lista atual, TTL 8 dias
//   {p}:cindex                  indice cifrado, TTL 8 dias
//   {p}:cphoto:{hash}           foto cifrada, TTL 8 dias (renovado em cada lista que a usa;
//                               as que saem da lista expiram sozinhas)
impl Store {
    /// Guarda um envio da Comunidade (bytes opacos). Devolve o id.
    pub async fn submit_community(&self, sender: &str, sealed: Vec<u8>) -> Result<u64, StoreError> {
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        let day_key = format!("{p}:csubday:{sender}:{}", today());
        let (count, stored): (u64, usize) = redis::pipe()
            .incr(&day_key, 1u64)
            .expire(&day_key, 2 * 86_400)
            .ignore()
            .llen(format!("{p}:csubs"))
            .query_async(&mut conn)
            .await?;
        if count > MAX_COMMUNITY_SUBMITS_PER_DAY {
            return Err(StoreError::CommunityLimit);
        }
        if stored >= MAX_COMMUNITY_STORED {
            return Err(StoreError::CommunityFull);
        }
        let id: u64 = conn.incr(format!("{p}:csub_seq"), 1u64).await?;
        let key = format!("{p}:csub:{id}");
        let mut value = now_ms().to_be_bytes().to_vec();
        value.extend_from_slice(&sealed);
        redis::pipe()
            .set(&key, value)
            .ignore()
            .expire(&key, COMMUNITY_SUB_TTL_SECONDS as i64)
            .ignore()
            .rpush(format!("{p}:csubs"), id)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(id)
    }

    /// Envios com id maior do que `after`, ate `max_bytes` estimados. `more`: ha mais.
    pub async fn list_community(
        &self,
        after: u64,
        max_bytes: usize,
    ) -> Result<(Vec<(u64, u64, Vec<u8>)>, bool), StoreError> {
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        let list = format!("{p}:csubs");
        let ids: Vec<u64> = conn.lrange(&list, 0, -1).await?;
        let mut out = Vec::new();
        let mut budget = 0usize;
        for id in ids.into_iter().filter(|id| *id > after) {
            let raw: Option<Vec<u8>> = conn.get(format!("{p}:csub:{id}")).await?;
            let Some(raw) = raw.filter(|r| r.len() > 8) else {
                conn.lrem::<_, _, ()>(&list, 0, id).await?;
                continue;
            };
            let est = raw.len().div_ceil(3) * 4 + 96; // base64 + JSON
            if !out.is_empty() && budget + est > max_bytes {
                return Ok((out, true));
            }
            budget += est;
            let at = u64::from_be_bytes(raw[..8].try_into().unwrap_or_default());
            out.push((id, at, raw[8..].to_vec()));
        }
        Ok((out, false))
    }

    pub async fn delete_community(&self, id: u64) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        redis::pipe()
            .del(format!("{p}:csub:{id}"))
            .ignore()
            .lrem(format!("{p}:csubs"), 0, id)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
    }

    /// Numero da lista atual (0 = sem lista).
    pub async fn community_version(&self) -> Result<u64, StoreError> {
        let mut conn = self.connection().await?;
        let v: Option<u64> = conn.get(format!("{}:cver", self.key_prefix)).await?;
        Ok(v.unwrap_or(0))
    }

    /// O indice atual e o seu numero (`None` = sem lista).
    pub async fn community_index(&self) -> Result<Option<(u64, Vec<u8>)>, StoreError> {
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        let (v, data): (Option<u64>, Option<Vec<u8>>) =
            redis::pipe().get(format!("{p}:cver")).get(format!("{p}:cindex")).query_async(&mut conn).await?;
        Ok(match (v, data) {
            (Some(v), Some(d)) if v > 0 && !d.is_empty() => Some((v, d)),
            _ => None,
        })
    }

    /// Uma foto da lista (`None` = nao ha).
    pub async fn community_photo(&self, hash: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let mut conn = self.connection().await?;
        Ok(conn.get(format!("{}:cphoto:{hash}", self.key_prefix)).await?)
    }

    /// Mais 8 dias para estas fotos. Devolve as que o relay nao tem.
    pub async fn community_keep(&self, hashes: &[String]) -> Result<Vec<String>, StoreError> {
        if hashes.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        let mut pipe = redis::pipe();
        for h in hashes {
            pipe.expire(format!("{p}:cphoto:{h}"), COMMUNITY_TTL_SECONDS as i64);
        }
        let kept: Vec<bool> = pipe.query_async(&mut conn).await?;
        Ok(hashes.iter().zip(kept).filter(|(_, k)| !k).map(|(h, _)| h.clone()).collect())
    }

    pub async fn community_put_photo(&self, hash: &str, data: Vec<u8>) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let key = format!("{}:cphoto:{hash}", self.key_prefix);
        redis::pipe()
            .set(&key, data)
            .ignore()
            .expire(&key, COMMUNITY_TTL_SECONDS as i64)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
    }

    /// A lista nova: indice e numero juntos (uma transacao), 8 dias.
    pub async fn community_publish(&self, version: u64, index: Vec<u8>) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        let ttl = COMMUNITY_TTL_SECONDS as i64;
        redis::pipe()
            .atomic()
            .set(format!("{p}:cindex"), index)
            .ignore()
            .expire(format!("{p}:cindex"), ttl)
            .ignore()
            .set(format!("{p}:cver"), version)
            .ignore()
            .expire(format!("{p}:cver"), ttl)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
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

// ------------------------------------------------------------ copia de recuperacao
//
// Bytes cifrados pela app (o relay nao os le). Chaves:
//   {p}:bkhead:{lookup}               "{geracao}:{partes}" da copia atual
//   {p}:bkpart:{lookup}:{geracao}:{i} uma parte
//   {p}:bkown:{lookup}                a conta dona (so ela substitui a copia)
//   {p}:bkof:{conta}                  o lookup da copia desta conta (para a apagar com ela)
impl Store {
    fn bk_keys(&self, lookup: &str) -> (String, String) {
        let p = &self.key_prefix;
        (format!("{p}:bkhead:{lookup}"), format!("{p}:bkown:{lookup}"))
    }

    fn bk_part(&self, lookup: &str, generation: &str, i: u32) -> String {
        format!("{}:bkpart:{lookup}:{generation}:{i}", self.key_prefix)
    }

    async fn bk_head(&self, conn: &mut redis::aio::ConnectionManager, lookup: &str) -> Result<Option<(String, u32)>, StoreError> {
        let head: Option<String> = conn.get(self.bk_keys(lookup).0).await?;
        Ok(head.and_then(|h| {
            let (g, n) = h.split_once(':')?;
            Some((g.to_string(), n.parse().ok()?))
        }))
    }

    /// Guarda uma parte; na ultima, troca a copia atual pela nova.
    pub async fn backup_put(
        &self,
        owner: &str,
        lookup: &str,
        generation: &str,
        part: u32,
        parts: u32,
        data: Vec<u8>,
    ) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let (head_key, own_key) = self.bk_keys(lookup);
        let current_owner: Option<String> = conn.get(&own_key).await?;
        if current_owner.as_deref().is_some_and(|o| o != owner) {
            return Err(StoreError::BackupNotOwner);
        }
        let key = self.bk_part(lookup, generation, part);
        redis::pipe()
            .set(&key, data)
            .ignore()
            .expire(&key, BACKUP_STAGE_TTL_SECONDS)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        if part + 1 != parts {
            return Ok(());
        }
        // Ultima parte: confirma que chegaram todas antes de trocar.
        for i in 0..parts {
            let v: Option<Vec<u8>> = conn.get(self.bk_part(lookup, generation, i)).await?;
            if v.is_none() {
                return Err(StoreError::BackupIncomplete);
            }
        }
        let old = self.bk_head(&mut conn, lookup).await?;
        let of_key = format!("{}:bkof:{owner}", self.key_prefix);
        let previous_lookup: Option<String> = conn.get(&of_key).await?;
        let mut pipe = redis::pipe();
        pipe.set(&head_key, format!("{generation}:{parts}")).ignore().expire(&head_key, BACKUP_TTL_SECONDS).ignore();
        pipe.set(&own_key, owner).ignore().expire(&own_key, BACKUP_TTL_SECONDS).ignore();
        pipe.set(&of_key, lookup).ignore().expire(&of_key, BACKUP_TTL_SECONDS).ignore();
        for i in 0..parts {
            pipe.expire(self.bk_part(lookup, generation, i), BACKUP_TTL_SECONDS).ignore();
        }
        if let Some((g, n)) = old.filter(|(g, _)| g != generation) {
            for i in 0..n {
                pipe.del(self.bk_part(lookup, &g, i)).ignore();
            }
        }
        pipe.query_async::<_, ()>(&mut conn).await?;
        // Palavras novas: a copia das palavras antigas desta conta deixa de servir.
        if let Some(prev) = previous_lookup.filter(|l| l != lookup) {
            self.delete_backup_lookup(&mut conn, &prev, owner).await?;
        }
        Ok(())
    }

    /// Uma parte da copia atual: `(partes, dados)`.
    pub async fn backup_get(&self, lookup: &str, part: u32) -> Result<Option<(u32, Vec<u8>)>, StoreError> {
        let mut conn = self.connection().await?;
        let Some((generation, parts)) = self.bk_head(&mut conn, lookup).await? else { return Ok(None) };
        if part >= parts {
            return Ok(None);
        }
        let data: Option<Vec<u8>> = conn.get(self.bk_part(lookup, &generation, part)).await?;
        Ok(data.map(|d| (parts, d)))
    }

    async fn delete_backup_lookup(
        &self,
        conn: &mut redis::aio::ConnectionManager,
        lookup: &str,
        owner: &str,
    ) -> Result<(), StoreError> {
        let (head_key, own_key) = self.bk_keys(lookup);
        let current_owner: Option<String> = conn.get(&own_key).await?;
        if current_owner.as_deref() != Some(owner) {
            return Ok(());
        }
        let head = self.bk_head(conn, lookup).await?;
        let mut pipe = redis::pipe();
        pipe.del(&head_key).ignore().del(&own_key).ignore();
        if let Some((g, n)) = head {
            for i in 0..n {
                pipe.del(self.bk_part(lookup, &g, i)).ignore();
            }
        }
        pipe.query_async::<_, ()>(conn).await?;
        Ok(())
    }

    /// Apaga a copia desta conta (pedido da app, conta apagada ou banida).
    pub async fn delete_backup(&self, owner: &str) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let of_key = format!("{}:bkof:{owner}", self.key_prefix);
        let lookup: Option<String> = conn.get(&of_key).await?;
        if let Some(l) = lookup {
            self.delete_backup_lookup(&mut conn, &l, owner).await?;
        }
        conn.del::<_, ()>(&of_key).await?;
        Ok(())
    }

    /// A app abriu (registou o bundle): a copia desta conta fica mais 90 dias.
    pub async fn renew_backup(&self, owner: &str) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let of_key = format!("{}:bkof:{owner}", self.key_prefix);
        let Some(lookup) = conn.get::<_, Option<String>>(&of_key).await? else { return Ok(()) };
        self.renew_backup_of(owner, &lookup).await
    }

    /// Como `renew_backup`, com o `lookup` ja lido (o registo le-o na mesma transacao).
    pub async fn renew_backup_of(&self, owner: &str, lookup: &str) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let of_key = format!("{}:bkof:{owner}", self.key_prefix);
        let lookup = lookup.to_string();
        let Some((generation, parts)) = self.bk_head(&mut conn, &lookup).await? else { return Ok(()) };
        let (head_key, own_key) = self.bk_keys(&lookup);
        let mut pipe = redis::pipe();
        for k in [&of_key, &head_key, &own_key] {
            pipe.expire(k, BACKUP_TTL_SECONDS).ignore();
        }
        for i in 0..parts {
            pipe.expire(self.bk_part(&lookup, &generation, i), BACKUP_TTL_SECONDS).ignore();
        }
        pipe.query_async::<_, ()>(&mut conn).await?;
        Ok(())
    }
}

// ------------------------------------------------------------ premium
//
//   {p}:premium:{conta}  ate quando (ms desde 1970, texto); expira sozinho 30 dias depois
//   {p}:premiums         lista das contas (as que expiraram saem ao listar)
//   {p}:premium_enforce  "1" = bloqueio ligado (0.9.21); ausente = desligado
impl Store {
    pub async fn premium_enforced(&self) -> Result<bool, StoreError> {
        let mut conn = self.connection().await?;
        let v: Option<String> = conn.get(format!("{}:premium_enforce", self.key_prefix)).await?;
        Ok(v.as_deref() == Some("1"))
    }

    pub async fn set_premium_enforced(&self, on: bool) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let key = format!("{}:premium_enforce", self.key_prefix);
        if on {
            conn.set::<_, _, ()>(&key, "1").await?;
        } else {
            conn.del::<_, ()>(&key).await?;
        }
        Ok(())
    }

    /// A acao paga pode seguir: bloqueio desligado, ou premium dentro do prazo.
    /// Num so pedido: o bloqueio esta ligado? ate quando e premium esta conta?
    pub async fn premium_allows(&self, user_id: &str) -> Result<bool, StoreError> {
        let mut conn = self.connection().await?;
        let (enforce, until): (Option<String>, Option<String>) = redis::pipe()
            .get(format!("{}:premium_enforce", self.key_prefix))
            .get(format!("{}:premium:{user_id}", self.key_prefix))
            .query_async(&mut conn)
            .await?;
        if enforce.as_deref() != Some("1") {
            return Ok(true);
        }
        Ok(until.and_then(|s| s.parse::<u64>().ok()).unwrap_or(0) > now_ms())
    }

    pub async fn premium_until(&self, user_id: &str) -> Result<u64, StoreError> {
        let mut conn = self.connection().await?;
        let v: Option<String> = conn.get(format!("{}:premium:{user_id}", self.key_prefix)).await?;
        Ok(v.and_then(|s| s.parse().ok()).unwrap_or(0))
    }

    /// Da premium ate `until_ms` (0 tira).
    pub async fn set_premium(&self, user_id: &str, until_ms: u64) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        let key = format!("{p}:premium:{user_id}");
        let list = format!("{p}:premiums");
        let mut pipe = redis::pipe();
        pipe.lrem(&list, 0, user_id).ignore();
        if until_ms == 0 {
            pipe.del(&key).ignore();
        } else {
            // Expira sozinho 30 dias depois do fim (sem fim: o Redis aceita ate ~292 milhoes de anos).
            let ttl = (until_ms.saturating_sub(now_ms()) / 1000).saturating_add(30 * 86_400).min(i64::MAX as u64) as i64;
            pipe.set(&key, until_ms.to_string()).ignore().expire(&key, ttl.max(1)).ignore().rpush(&list, user_id).ignore();
        }
        pipe.query_async::<_, ()>(&mut conn).await?;
        Ok(())
    }

    pub async fn list_premium(&self) -> Result<Vec<crate::protocol::PremiumEntry>, StoreError> {
        let mut conn = self.connection().await?;
        let list = format!("{}:premiums", self.key_prefix);
        let ids: Vec<String> = conn.lrange(&list, 0, -1).await?;
        let mut out = Vec::new();
        for id in ids {
            let until = self.premium_until(&id).await?;
            if until == 0 {
                conn.lrem::<_, _, ()>(&list, 0, &id).await?;
            } else {
                out.push(crate::protocol::PremiumEntry { user_id: id, until_ms: until });
            }
        }
        Ok(out)
    }
}

#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub id: u64,
    pub sealed_from: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

/// Quem envia uma mensagem (`Store::enqueue`), para as verificacoes da fila.
#[derive(Debug, Clone, Copy)]
pub enum Sender<'a> {
    /// Conta deste relay por verificar: `EXISTS` do bundle no mesmo pedido da escrita.
    Check(&'a str),
    /// Conta deste relay ja verificada nesta ligacao (na autenticacao ou no registo).
    Registered,
    /// Outro relay da federacao: ja verificou o remetente do lado dele.
    Peer,
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

/// Federacao (fase 1, `federation.rs`): o que este relay sabe de uma conta.
impl Store {
    fn fed_pv_key(&self, user_id: &str) -> String {
        format!("{}:fed:pv:{user_id}", self.key_prefix)
    }

    fn fed_pv_list(&self) -> String {
        format!("{}:fed:verificar", self.key_prefix)
    }

    /// Estado de uma conta NESTE relay, para responder a outro relay. Uma ida ao Redis.
    pub async fn fed_estado(&self, user_id: &str) -> Result<crate::federation::Estado, StoreError> {
        use crate::federation::Estado;
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        type Linha = (Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>);
        let (bundle, pv, ban, deleted): Linha = redis::pipe()
            .get(format!("{p}:bundle:{user_id}"))
            .get(self.fed_pv_key(user_id))
            .get(format!("{p}:ban:{user_id}"))
            .get(self.deleted_key(user_id))
            .query_async(&mut conn)
            .await?;
        Ok(match (bundle.is_some(), pv.is_some(), ban.is_some(), deleted.is_some()) {
            (_, _, true, _) => Estado::Banida,
            (true, true, _, _) => Estado::TenhoPorVerificar,
            (true, false, _, _) => Estado::Tenho,
            (false, _, _, true) => Estado::Apagada,
            _ => Estado::NaoTenho,
        })
    }

    /// Conta registada sem resposta de algum relay (opcao A1): fica para verificar
    /// quando ele voltar. A marca dura o mesmo que o bundle.
    pub async fn fed_marcar_por_verificar(&self, user_id: &str) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let (key, list) = (self.fed_pv_key(user_id), self.fed_pv_list());
        redis::pipe()
            .set(&key, 1u8)
            .ignore()
            .expire(&key, BUNDLE_TTL_SECONDS as i64)
            .ignore()
            .lrem(&list, 0, user_id)
            .ignore()
            .rpush(&list, user_id)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
    }

    /// As primeiras `max` contas por verificar.
    pub async fn fed_por_verificar(&self, max: usize) -> Result<Vec<String>, StoreError> {
        let mut conn = self.connection().await?;
        Ok(conn.lrange(self.fed_pv_list(), 0, max.max(1) as isize - 1).await?)
    }

    /// Verificada: nenhum outro relay tem a conta.
    pub async fn fed_verificado(&self, user_id: &str) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        redis::pipe()
            .del(self.fed_pv_key(user_id))
            .ignore()
            .lrem(self.fed_pv_list(), 0, user_id)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
    }

    /// A conta ja estava noutro relay: este deixa de a ter (bundle, one-time prekeys e
    /// campainha). A fila fica (expira sozinha; a fase 2 encaminha-a). Sem marca de conta
    /// apagada: a conta continua viva, so que no outro relay.
    pub async fn fed_largar(&self, user_id: &str) -> Result<(), StoreError> {
        self.set_push_token(user_id, None).await?;
        let mut conn = self.connection().await?;
        let p = &self.key_prefix;
        redis::pipe()
            .del(format!("{p}:bundle:{user_id}"))
            .ignore()
            .del(format!("{p}:otpk:{user_id}"))
            .ignore()
            .del(self.fed_pv_key(user_id))
            .ignore()
            .lrem(self.fed_pv_list(), 0, user_id)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
    }
}
