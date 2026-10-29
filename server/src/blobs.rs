//! Anexos (imagens e GIFs) guardados no disco do relay, no maximo 24 horas.
//!
//! O relay continua cego: o ficheiro chega JA CIFRADO pelo telemovel de quem
//! envia, com uma chave aleatoria que so viaja dentro de uma mensagem do Double
//! Ratchet. O relay guarda bytes opacos e so sabe o tamanho (arredondado pelo
//! cliente para esconder o valor exato) e quando foi enviado e descarregado.
//!
//! Fluxo:
//!   1. `BlobCreate { size }`  -> `BlobCreated { blob_id, token }` (id e token aleatorios)
//!   2. `BlobPut { blob_id, token, offset, data }`, por ordem, ate `size` bytes.
//!      Repetir um bloco ja recebido e inofensivo (retomar depois de uma falha
//!      de rede); `BlobInfo` diz quantos bytes ja chegaram.
//!   3. O destinatario faz `BlobGet { blob_id, offset }` ate ao fim e depois
//!      `BlobDelete { blob_id }`. O ficheiro e apagado logo.
//!   4. O que ninguem descarregar e apagado ao fim de 24 horas (TTL no Redis e
//!      limpeza periodica do disco, `sweep_blobs`).
//!
//! Privacidade: o `blob_id` e uma capacidade de 128 bits (quem o conhece pode ler
//! e apagar). O relay NAO guarda quem criou cada ficheiro: so o hash do token de
//! escrita. Para limitar abusos guarda, durante 2 dias, quantos bytes cada
//! utilizador enviou por dia (`blobquota:<user>:<dia>`), sem ligacao aos ficheiros.

use crate::store::{Store, StoreError};
use base64::Engine;
use redis::AsyncCommands;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Bytes de ficheiro por pedido/resposta. Em base64 dentro do JSON fica com
/// ~55 KB, o que cabe numa mensagem Noise (65 535 bytes).
pub const BLOB_CHUNK: usize = 40 * 1024;
/// Maior ficheiro aceite: 25 MiB de imagem + arredondamento + etiquetas AEAD.
pub const MAX_BLOB_BYTES: u64 = 27 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct BlobConfig {
    /// Pasta dos ficheiros (so o relay le e escreve, 0700).
    pub dir: PathBuf,
    /// Quanto tempo um ficheiro espera para ser descarregado (24 h).
    pub ttl: Duration,
    /// Espaco maximo ocupado por anexos no disco.
    pub max_total_bytes: u64,
    /// Bytes que cada utilizador pode enviar por dia (anti-abuso).
    pub max_user_bytes_per_day: u64,
    /// Espaco que fica sempre livre no disco (para o sistema, o Redis e os registos):
    /// abaixo disto, o relay recusa anexos novos mesmo sem chegar a `max_total_bytes`.
    pub min_free_bytes: u64,
    /// Folga da limpeza: um ficheiro sem registo no Redis so e apagado depois
    /// disto (o registo e criado logo a seguir ao ficheiro), e um ficheiro so e
    /// apagado por idade depois de `ttl + grace`.
    pub grace: Duration,
}

impl BlobConfig {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            ttl: Duration::from_secs(24 * 60 * 60),
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            max_user_bytes_per_day: 1024 * 1024 * 1024,
            grace: Duration::from_secs(600),
            min_free_bytes: 1024 * 1024 * 1024,
        }
    }
}

pub(crate) struct Blobs {
    cfg: BlobConfig,
    /// Bytes reservados no disco (tamanho declarado de cada ficheiro).
    /// Recalculado por `sweep_blobs`.
    used: AtomicU64,
    /// As escritas sao curtas (40 KB); uma de cada vez evita dois pedidos a
    /// escrever no mesmo ficheiro ao mesmo tempo.
    write_lock: tokio::sync::Mutex<()>,
}

#[derive(Debug, Error)]
pub enum BlobError {
    #[error("anexos desativados neste relay")]
    Disabled,
    #[error("tamanho invalido")]
    BadSize,
    #[error("sem espaco para anexos")]
    NoSpace,
    #[error("limite diario de anexos atingido")]
    Quota,
    #[error("anexo inexistente ou expirado")]
    NotFound,
    #[error("sem permissao")]
    Forbidden,
    #[error("posicao invalida")]
    BadOffset,
    #[error("anexo ainda incompleto")]
    Incomplete,
    #[error("pedido invalido")]
    BadRequest,
    #[error("erro de disco")]
    Io(#[from] std::io::Error),
    #[error("erro interno: {0}")]
    Store(#[from] StoreError),
}

impl From<redis::RedisError> for BlobError {
    fn from(e: redis::RedisError) -> Self {
        BlobError::Store(StoreError::Redis(e))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobStatus {
    pub size: u64,
    pub received: u64,
    pub complete: bool,
}

pub fn is_valid_blob_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn random_hex16() -> String {
    let mut b = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|x| format!("{x:02x}")).collect()
}

/// Comparacao sem atalhos (o tempo nao depende de onde os textos diferem).
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(crate) fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

impl Blobs {
    pub(crate) fn new(cfg: BlobConfig) -> std::io::Result<Self> {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        b.create(&cfg.dir)?;
        Ok(Self { cfg, used: AtomicU64::new(0), write_lock: tokio::sync::Mutex::new(()) })
    }

    fn path(&self, id: &str) -> PathBuf {
        self.cfg.dir.join(id)
    }
}

struct Meta {
    size: u64,
    token_hash: String,
    complete: bool,
}

impl Store {
    fn blobs(&self) -> Result<&Blobs, BlobError> {
        self.blobs.as_ref().ok_or(BlobError::Disabled)
    }

    fn blob_key(&self, id: &str) -> String {
        format!("{}:blob:{}", self.key_prefix, id)
    }

    async fn blob_meta(&self, id: &str) -> Result<Meta, BlobError> {
        if !is_valid_blob_id(id) {
            return Err(BlobError::BadRequest);
        }
        let mut conn = self.connection().await?;
        let m: std::collections::HashMap<String, String> = conn.hgetall(self.blob_key(id)).await?;
        let size = m.get("size").and_then(|s| s.parse().ok()).ok_or(BlobError::NotFound)?;
        Ok(Meta {
            size,
            token_hash: m.get("token").cloned().unwrap_or_default(),
            complete: m.get("complete").map(|v| v == "1").unwrap_or(false),
        })
    }

    /// Reserva espaco para um ficheiro de `size` bytes enviado por `owner`.
    pub async fn blob_create(&self, owner: &str, size: u64) -> Result<(String, String), BlobError> {
        let blobs = self.blobs()?;
        if size == 0 || size > MAX_BLOB_BYTES {
            return Err(BlobError::BadSize);
        }
        if free_bytes(&blobs.cfg.dir).is_some_and(|free| free < size.saturating_add(blobs.cfg.min_free_bytes)) {
            return Err(BlobError::NoSpace);
        }
        let prev = blobs.used.fetch_add(size, Ordering::SeqCst);
        if prev.saturating_add(size) > blobs.cfg.max_total_bytes {
            blobs.used.fetch_sub(size, Ordering::SeqCst);
            return Err(BlobError::NoSpace);
        }
        let release = |e: BlobError| {
            blobs.used.fetch_sub(size, Ordering::SeqCst);
            e
        };
        let mut conn = self.connection().await.map_err(|e| release(e.into()))?;
        let day = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0);
        let qkey = format!("{}:blobquota:{}:{}", self.key_prefix, owner, day);

        let id = random_hex16();
        let token = random_hex16();
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        opts.open(blobs.path(&id)).map_err(|e| release(e.into()))?;
        // Quota do dia e registo do anexo num so pedido ao Redis (antes eram quatro, um a
        // um). Se a quota passar do limite, desfaz: devolve os bytes ao contador (DECRBY) e
        // apaga o registo, que ninguem chegou a ver (o id e o token nao sairam daqui).
        let key = self.blob_key(&id);
        let res: Result<(u64,), redis::RedisError> = redis::pipe()
            .incr(&qkey, size)
            .expire(&qkey, 2 * 86_400)
            .ignore()
            .hset_multiple(&key, &[("size", size.to_string()), ("token", token_hash(&token)), ("complete", "0".to_string())])
            .ignore()
            .expire(&key, blobs.cfg.ttl.as_secs() as i64)
            .ignore()
            .query_async(&mut conn)
            .await;
        match res {
            // Erro do Redis: nao se sabe que comandos correram, por isso o contador nao e
            // mexido (como antes, quando o EXPIRE falhava depois do INCRBY).
            Err(e) => {
                let _ = std::fs::remove_file(blobs.path(&id));
                return Err(release(e.into()));
            }
            Ok((total,)) if total > blobs.cfg.max_user_bytes_per_day => {
                let _: Result<(), _> = redis::pipe().decr(&qkey, size).ignore().del(&key).ignore().query_async(&mut conn).await;
                let _ = std::fs::remove_file(blobs.path(&id));
                return Err(release(BlobError::Quota));
            }
            Ok(_) => {}
        }
        Ok((id, token))
    }

    /// Acrescenta um bloco. Tem de comecar onde o ficheiro acaba; um bloco que
    /// ja tinha chegado (reenvio depois de uma falha) e aceite sem mudar nada.
    pub async fn blob_put(&self, id: &str, token: &str, offset: u64, data_b64: &str) -> Result<BlobStatus, BlobError> {
        let blobs = self.blobs()?;
        let meta = self.blob_meta(id).await?;
        if !same(&token_hash(token), &meta.token_hash) {
            return Err(BlobError::Forbidden);
        }
        if data_b64.len() > BLOB_CHUNK.div_ceil(3) * 4 {
            return Err(BlobError::BadSize);
        }
        let data = b64().decode(data_b64).map_err(|_| BlobError::BadRequest)?;
        if data.is_empty() || data.len() > BLOB_CHUNK {
            return Err(BlobError::BadSize);
        }
        let end = offset.checked_add(data.len() as u64).ok_or(BlobError::BadOffset)?;
        if end > meta.size {
            return Err(BlobError::BadSize);
        }
        let _guard = blobs.write_lock.lock().await;
        let path = blobs.path(id);
        let current = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(BlobError::NotFound),
            Err(e) => return Err(e.into()),
        };
        if end <= current {
            return Ok(BlobStatus { size: meta.size, received: current, complete: current == meta.size });
        }
        if offset != current {
            return Err(BlobError::BadOffset);
        }
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(&path)?;
            f.write_all(&data)?;
            if end == meta.size {
                f.sync_all()?;
            }
        }
        if end == meta.size {
            let mut conn = self.connection().await?;
            conn.hset::<_, _, _, ()>(self.blob_key(id), "complete", "1").await?;
        }
        Ok(BlobStatus { size: meta.size, received: end, complete: end == meta.size })
    }

    /// Quantos bytes ja chegaram (para retomar um envio interrompido).
    pub async fn blob_info(&self, id: &str) -> Result<BlobStatus, BlobError> {
        let blobs = self.blobs()?;
        let meta = self.blob_meta(id).await?;
        let received = std::fs::metadata(blobs.path(id)).map(|m| m.len()).map_err(|_| BlobError::NotFound)?;
        Ok(BlobStatus { size: meta.size, received, complete: meta.complete && received == meta.size })
    }

    /// Um bloco de um ficheiro completo, a partir de `offset`.
    pub async fn blob_get(&self, id: &str, offset: u64) -> Result<(u64, Vec<u8>), BlobError> {
        let blobs = self.blobs()?;
        let meta = self.blob_meta(id).await?;
        if !meta.complete {
            return Err(BlobError::Incomplete);
        }
        if offset >= meta.size {
            return Err(BlobError::BadOffset);
        }
        let len = (meta.size - offset).min(BLOB_CHUNK as u64) as usize;
        let path = blobs.path(id);
        let data = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = std::fs::File::open(path)?;
            f.seek(SeekFrom::Start(offset))?;
            let mut buf = vec![0u8; len];
            f.read_exact(&mut buf)?;
            Ok(buf)
        })
        .await
        .map_err(|_| BlobError::Io(std::io::Error::other("tarefa")))?
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { BlobError::NotFound } else { BlobError::Io(e) })?;
        Ok((meta.size, data))
    }

    /// Apaga ja o ficheiro (o destinatario ja o tem). Apagar um que ja nao
    /// existe nao e erro.
    pub async fn blob_delete(&self, id: &str) -> Result<(), BlobError> {
        let blobs = self.blobs()?;
        if !is_valid_blob_id(id) {
            return Err(BlobError::BadRequest);
        }
        let size = match self.blob_meta(id).await {
            Ok(m) => Some(m.size),
            Err(BlobError::NotFound) => None,
            Err(e) => return Err(e),
        };
        let mut conn = self.connection().await?;
        let removed: u64 = conn.del(self.blob_key(id)).await?;
        let _ = std::fs::remove_file(blobs.path(id));
        if let (Some(size), 1) = (size, removed) {
            // Subtrai sem passar abaixo de zero (ciclo compare-and-swap: igual em
            // qualquer versao do Rust, sem o `fetch_update` que o Rust novo da como antiquado).
            let mut current = blobs.used.load(Ordering::SeqCst);
            while let Err(actual) =
                blobs.used.compare_exchange_weak(current, current.saturating_sub(size), Ordering::SeqCst, Ordering::SeqCst)
            {
                current = actual;
            }
        }
        Ok(())
    }

    /// Limpeza periodica: apaga ficheiros expirados ou sem registo no Redis e
    /// recalcula o espaco ocupado. Devolve (ficheiros apagados, bytes reservados).
    pub async fn sweep_blobs(&self) -> Result<(u64, u64), BlobError> {
        let blobs = self.blobs()?;
        let entries: Vec<(String, PathBuf, Duration)> = {
            let mut v = Vec::new();
            for e in std::fs::read_dir(&blobs.cfg.dir)? {
                let e = e?;
                let name = e.file_name().to_string_lossy().into_owned();
                if !is_valid_blob_id(&name) {
                    continue; // nao e nosso: nao mexe
                }
                let age = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .unwrap_or_default();
                v.push((name, e.path(), age));
            }
            v
        };
        let mut conn = self.connection().await?;
        let mut removed = 0u64;
        let mut used = 0u64;
        for (id, path, age) in entries {
            let key = self.blob_key(&id);
            let size: Option<u64> = conn.hget(&key, "size").await?;
            let too_old = age > blobs.cfg.ttl + blobs.cfg.grace;
            let orphan = size.is_none() && age >= blobs.cfg.grace;
            if too_old || orphan {
                if remove(&path) {
                    removed += 1;
                }
                let _: u64 = conn.del(&key).await?;
            } else {
                used += size.unwrap_or(0);
            }
        }
        blobs.used.store(used, Ordering::SeqCst);
        Ok((removed, used))
    }
}

/// Espaco livre no disco onde esta `dir` (o que um processo sem privilegios pode usar).
#[cfg(unix)]
fn free_bytes(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    // SAFETY: `statvfs` so escreve na estrutura que lhe passamos; o caminho e uma
    // string C valida que vive ate ao fim da chamada.
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut s) } != 0 {
        return None;
    }
    Some((s.f_bavail as u64).saturating_mul(s.f_frsize as u64))
}

#[cfg(not(unix))]
fn free_bytes(_dir: &Path) -> Option<u64> {
    None
}

fn remove(path: &Path) -> bool {
    std::fs::remove_file(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn espaco_livre_do_disco() {
        let free = free_bytes(&std::env::temp_dir());
        assert!(free.is_some_and(|f| f > 0), "{free:?}");
        assert!(free_bytes(Path::new("/nao/existe/mesmo")).is_none());
    }

    #[test]
    fn ids_e_comparacao() {
        assert!(is_valid_blob_id(&random_hex16()));
        assert!(!is_valid_blob_id("../../etc/passwd"));
        assert!(!is_valid_blob_id(&"A".repeat(32)));
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("abc", "ab"));
        // Um bloco inteiro em base64 cabe numa mensagem Noise, com o JSON a volta.
        let json = format!(
            r#"{{"BlobPut":{{"blob_id":"{}","token":"{}","offset":28311552,"data":"{}"}}}}"#,
            "a".repeat(32),
            "b".repeat(32),
            b64().encode(vec![0u8; BLOB_CHUNK])
        );
        assert!(json.len() < 65_535 - 16, "{}", json.len());
    }
}
