//! Copias de recuperacao no disco do relay, numa pasta so delas (nada a ver com os anexos).
//!
//! Os bytes ja chegam cifrados pela app. No Redis fica so o indice (`bkhead`, `bkown`, `bkof`),
//! com os prazos; o conteudo vai para um ficheiro por copia: `{pasta}/{2 primeiros}/{lookup}`.
//! As partes a meio do envio continuam no Redis (1 hora).
//!
//! Regras para nao se perder nenhuma copia:
//!   * so se responde "guardado" depois de o ficheiro estar no disco (fsync);
//!   * a troca e atomica (ficheiro temporario e rename);
//!   * uma copia antiga, ainda no Redis, so sai de la depois de estar no disco;
//!   * a limpeza so apaga um ficheiro parado ha mais de 90 dias E sem indice no Redis;
//!   * um erro de disco nunca vira "nao ha copia".
//!
//! Ficheiro: `SBK1`, numero de partes (1 byte), tamanho de cada parte (4 bytes LE) e as partes.

use crate::store::{Store, StoreError, BACKUP_TTL_SECONDS};
use redis::AsyncCommands;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::{Mutex, OwnedMutexGuard};

const MAGIC: &[u8; 4] = b"SBK1";
/// Um ficheiro temporario com mais disto ficou de um envio interrompido.
const TMP_MAX_AGE: Duration = Duration::from_secs(60 * 60);

/// A vez de mexer nos ficheiros. Quem grava ou apaga entrega-a ao disco e recebe-a de volta: se
/// a ligacao for fechada a meio, a vez so passa quando o disco acabar.
pub(crate) type Turn = OwnedMutexGuard<()>;

#[derive(Clone, Debug)]
pub struct BackupConfig {
    /// Pasta das copias (so o relay le e escreve, 0700).
    pub dir: PathBuf,
    /// Espaco que fica sempre livre no disco: abaixo disto nao se escrevem copias. Maior que o
    /// dos anexos, para as copias nunca lhes tirarem o lugar.
    pub min_free_bytes: u64,
}

impl BackupConfig {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into(), min_free_bytes: 2 * 1024 * 1024 * 1024 }
    }
}

pub(crate) struct Backups {
    cfg: BackupConfig,
    /// Escrever, mover do Redis, apagar e limpar: um de cada vez. Sao raros e curtos, e assim
    /// uma copia antiga a ser movida nunca passa por cima de uma nova.
    lock: Arc<Mutex<()>>,
}

impl Backups {
    /// Cria a pasta e confirma que se consegue escrever nela.
    pub(crate) fn new(cfg: BackupConfig) -> io::Result<Self> {
        create_private_dir(&cfg.dir)?;
        let probe = cfg.dir.join("teste.tmp");
        std::fs::write(&probe, b"ok")?;
        std::fs::remove_file(&probe)?;
        Ok(Self { cfg, lock: Arc::default() })
    }

    pub(crate) async fn turn(&self) -> Turn {
        self.lock.clone().lock_owned().await
    }

    /// O trabalho de disco fica fora das threads do relay.
    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(&BackupConfig) -> io::Result<T> + Send + 'static,
    ) -> Result<T, StoreError> {
        let cfg = self.cfg.clone();
        let result = tokio::task::spawn_blocking(move || work(&cfg)).await.unwrap_or_else(|_| Err(io::Error::other("tarefa")));
        result.map_err(|e| {
            // Sem o caminho: o nome do ficheiro identifica a copia.
            eprintln!("copias de recuperacao: erro de disco ({e})");
            StoreError::BackupDisk(e)
        })
    }

    /// Grava a copia inteira. So devolve `Ok` com ela no disco.
    pub(crate) async fn write(&self, turn: Turn, lookup: &str, parts: Vec<Vec<u8>>) -> Result<Turn, StoreError> {
        let lookup = lookup.to_string();
        self.blocking(move |cfg| write_file(cfg, &lookup, &parts).map(|()| turn)).await
    }

    /// As partes da copia, ou `None` se nao ha ficheiro. Um ficheiro estragado e um erro.
    pub(crate) async fn read(&self, lookup: &str) -> Result<Option<Vec<Vec<u8>>>, StoreError> {
        let lookup = lookup.to_string();
        self.blocking(move |cfg| match std::fs::read(file_path(&cfg.dir, &lookup)?) {
            Ok(bytes) => decode(&bytes).map(Some).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "ficheiro estragado")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        })
        .await
    }

    /// Os lookups com ficheiro na pasta `{prefixo}` (dois hex).
    pub(crate) async fn list(&self, prefixo: u8) -> Result<Vec<String>, StoreError> {
        self.blocking(move |cfg| {
            let entries = match std::fs::read_dir(cfg.dir.join(format!("{prefixo:02x}"))) {
                Ok(e) => e,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(e) => return Err(e),
            };
            let mut out = Vec::new();
            for entry in entries {
                let name = entry?.file_name().to_string_lossy().into_owned();
                if crate::protocol::is_hex_of_len(&name, 64) {
                    out.push(name);
                }
            }
            Ok(out)
        })
        .await
    }

    /// Apaga o ficheiro. Um que nao exista nao e erro.
    pub(crate) async fn remove(&self, turn: Turn, lookup: &str) -> Result<Turn, StoreError> {
        let lookup = lookup.to_string();
        self.blocking(move |cfg| {
            let path = file_path(&cfg.dir, &lookup)?;
            match std::fs::remove_file(&path) {
                Ok(()) => path.parent().map_or(Ok(()), sync_dir)?,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            Ok(turn)
        })
        .await
    }

    /// A data do ficheiro passa a ser `ago` atras (zero = agora). E dela que a limpeza conta os
    /// 90 dias. Um ficheiro que nao exista nao e erro.
    pub(crate) async fn set_age(&self, lookup: &str, ago: Duration) -> Result<(), StoreError> {
        let lookup = lookup.to_string();
        self.blocking(move |cfg| match std::fs::OpenOptions::new().write(true).open(file_path(&cfg.dir, &lookup)?) {
            Ok(f) => f.set_modified(SystemTime::now() - ago),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        })
        .await
    }
}

/// `lookup` vai para o nome do ficheiro: so 64 caracteres hexadecimais.
fn file_path(dir: &Path, lookup: &str) -> io::Result<PathBuf> {
    if !crate::protocol::is_hex_of_len(lookup, 64) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "lookup invalido"));
    }
    Ok(dir.join(&lookup[..2]).join(lookup))
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir)
}

/// Grava no disco a entrada nova (ou apagada) de uma pasta.
fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    std::fs::File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

fn encode(parts: &[Vec<u8>]) -> io::Result<Vec<u8>> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "copia demasiado grande");
    let mut out = Vec::from(&MAGIC[..]);
    out.push(u8::try_from(parts.len()).map_err(|_| invalid())?);
    for p in parts {
        out.extend_from_slice(&u32::try_from(p.len()).map_err(|_| invalid())?.to_le_bytes());
    }
    for p in parts {
        out.extend_from_slice(p);
    }
    Ok(out)
}

fn decode(bytes: &[u8]) -> Option<Vec<Vec<u8>>> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let (&count, rest) = rest.split_first()?;
    let header = usize::from(count) * 4;
    let (lengths, mut data) = (rest.get(..header)?, rest.get(header..)?);
    let mut parts = Vec::with_capacity(usize::from(count));
    for l in lengths.chunks_exact(4) {
        let len = u32::from_le_bytes(l.try_into().ok()?) as usize;
        parts.push(data.get(..len)?.to_vec());
        data = data.get(len..)?;
    }
    (data.is_empty() && count > 0).then_some(parts)
}

fn write_file(cfg: &BackupConfig, lookup: &str, parts: &[Vec<u8>]) -> io::Result<()> {
    let path = file_path(&cfg.dir, lookup)?;
    let bytes = encode(parts)?;
    if crate::blobs::free_bytes(&cfg.dir).is_some_and(|free| free < cfg.min_free_bytes.saturating_add(bytes.len() as u64)) {
        return Err(io::Error::new(io::ErrorKind::StorageFull, "sem espaco no disco"));
    }
    let sub = cfg.dir.join(&lookup[..2]);
    if !sub.is_dir() {
        create_private_dir(&sub)?;
        sync_dir(&cfg.dir)?;
    }
    let mut random = [0u8; 8];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut random);
    let tmp = sub.join(format!("{lookup}.{:016x}.tmp", u64::from_le_bytes(random)));
    let written = (|| {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)?;
        sync_dir(&sub)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Candidatos a apagar: copias paradas ha mais de 90 dias (`Some(lookup)`, falta confirmar que
/// nao tem indice) e temporarios abandonados (`None`). Ficheiros com outros nomes ficam.
fn old_files(dir: &Path) -> io::Result<Vec<(Option<String>, PathBuf)>> {
    let mut out = Vec::new();
    for sub in std::fs::read_dir(dir)? {
        let sub = sub?;
        if !sub.file_type()?.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(sub.path())? {
            let file = file?;
            let name = file.file_name().to_string_lossy().into_owned();
            // Apagado entretanto ou com data no futuro: fica para a proxima.
            let Some(age) = file.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()) else { continue };
            if name.ends_with(".tmp") {
                if age > TMP_MAX_AGE {
                    out.push((None, file.path()));
                }
            } else if crate::protocol::is_hex_of_len(&name, 64) && age > Duration::from_secs(BACKUP_TTL_SECONDS as u64) {
                out.push((Some(name), file.path()));
            }
        }
    }
    Ok(out)
}

impl Store {
    /// Move para o disco a copia que ainda esta no Redis (so sai de la depois de gravada); se ja
    /// esta no disco, marca o ficheiro como usado agora.
    pub(crate) async fn backup_move_or_touch(
        &self,
        backups: &Backups,
        conn: &mut redis::aio::ConnectionManager,
        lookup: &str,
    ) -> Result<(), StoreError> {
        let Some((generation, _)) = self.bk_head(conn, lookup).await? else { return Ok(()) };
        if !conn.exists::<_, bool>(self.bk_part(lookup, &generation, 0)).await? {
            return backups.set_age(lookup, Duration::ZERO).await;
        }
        let turn = backups.turn().await;
        // Relido ja na nossa vez: entretanto pode ter chegado uma copia nova ou ter sido apagada.
        let Some((generation, parts)) = self.bk_head(conn, lookup).await? else { return Ok(()) };
        let mut chunks = Vec::with_capacity(parts as usize);
        for i in 0..parts {
            match conn.get::<_, Option<Vec<u8>>>(self.bk_part(lookup, &generation, i)).await? {
                Some(d) => chunks.push(d),
                // No Redis so ha parte (um reenvio a meio): a copia inteira e a do disco.
                None => return backups.set_age(lookup, Duration::ZERO).await,
            }
        }
        let _turn = backups.write(turn, lookup, chunks).await?;
        let mut pipe = redis::pipe();
        for i in 0..parts {
            pipe.del(self.bk_part(lookup, &generation, i)).ignore();
        }
        pipe.query_async::<_, ()>(conn).await?;
        Ok(())
    }

    /// Limpeza das copias no disco. Um ficheiro so sai se estiver parado ha mais de 90 dias E ja
    /// nao tiver indice no Redis: um Redis esvaziado por engano nao leva os ficheiros logo.
    /// Devolve quantos apagou.
    pub async fn sweep_backups(&self) -> Result<u64, StoreError> {
        let Some(backups) = &self.backups else { return Ok(0) };
        let candidates = backups.blocking(|cfg| old_files(&cfg.dir)).await?;
        let mut conn = self.connection().await?;
        let mut removed = 0;
        for (lookup, path) in candidates {
            let _turn = backups.turn().await;
            if let Some(lookup) = lookup {
                if conn.exists::<_, bool>(self.bk_keys(&lookup).0).await? {
                    continue;
                }
            }
            if std::fs::remove_file(path).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formato_ida_e_volta_e_estragados() {
        let parts = vec![vec![1u8; 40_000], vec![2u8; 5], vec![]];
        let bytes = encode(&parts).unwrap();
        assert_eq!(decode(&bytes), Some(parts));
        assert_eq!(decode(&bytes[..bytes.len() - 1]), None, "cortado");
        assert_eq!(decode(&[bytes.clone(), vec![0]].concat()), None, "a mais");
        assert_eq!(decode(b"SBK1\x00"), None, "sem partes");
        assert_eq!(decode(b"SBK1\x08"), None, "cabecalho cortado");
        assert_eq!(decode(b""), None);
    }

    #[test]
    fn o_nome_do_ficheiro_nao_sai_da_pasta() {
        let dir = Path::new("/var/lib/qt/copias");
        assert_eq!(file_path(dir, &"ab".repeat(32)).unwrap(), dir.join("ab").join("ab".repeat(32)));
        for bad in ["../../etc/passwd", "", &"AB".repeat(32), &"a".repeat(63)] {
            assert!(file_path(dir, bad).is_err(), "{bad}");
        }
    }
}
