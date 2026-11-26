//! Copia de recuperacao no disco (`backups.rs`): o que fica onde, copias antigas que ainda estao
//! no Redis, falhas de disco, apagar, banir e a limpeza. Precisa de um Redis em 127.0.0.1:6379.

use qt_server::store::{StoreError, BACKUP_TTL_SECONDS};
use qt_server::{BackupConfig, Store};
use redis::AsyncCommands;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const REDIS: &str = "redis://127.0.0.1:6379";
const DAY: u64 = 86_400;
const G1: &str = "1111111111111111";
const G2: &str = "2222222222222222";

/// Um relay de teste: prefixo no Redis e pasta de copias proprios.
struct Lugar {
    prefix: String,
    dir: PathBuf,
}

impl Lugar {
    fn new(nome: &str) -> Self {
        let prefix = format!("disco-{}-{nome}", std::process::id());
        let dir = std::env::temp_dir().join(format!("qt-copias-{prefix}"));
        let _ = std::fs::remove_dir_all(&dir);
        Self { prefix, dir }
    }

    /// Como o relay de agora: copias no disco.
    fn store(&self) -> Store {
        self.store_com_minimo_livre(0)
    }

    fn store_com_minimo_livre(&self, min_free_bytes: u64) -> Store {
        let mut s = self.store_so_redis();
        let mut cfg = BackupConfig::new(&self.dir);
        cfg.min_free_bytes = min_free_bytes;
        s.enable_backups(cfg).unwrap();
        s
    }

    /// Como o relay da versao anterior: copias no Redis.
    fn store_so_redis(&self) -> Store {
        Store::with_prefix(REDIS, &self.prefix).unwrap()
    }

    fn file(&self, lookup: &str) -> PathBuf {
        self.dir.join(&lookup[..2]).join(lookup)
    }

    async fn redis(&self) -> redis::aio::MultiplexedConnection {
        redis::Client::open(REDIS).unwrap().get_multiplexed_async_connection().await.unwrap()
    }

    async fn existe(&self, sufixo: String) -> bool {
        self.redis().await.exists(format!("{}:{sufixo}", self.prefix)).await.unwrap()
    }

    async fn ttl(&self, sufixo: String) -> i64 {
        self.redis().await.ttl(format!("{}:{sufixo}", self.prefix)).await.unwrap()
    }

    /// Como se os 90 dias do indice tivessem passado (ou o Redis fosse esvaziado).
    async fn esquecer_indice(&self, lookup: &str) {
        let _: () = self.redis().await.del(format!("{}:bkhead:{lookup}", self.prefix)).await.unwrap();
    }
}

fn id(c: char) -> String {
    c.to_string().repeat(64)
}

async fn put(s: &Store, owner: &str, lookup: &str, generation: &str, chunks: &[Vec<u8>]) -> Result<(), StoreError> {
    for (i, d) in chunks.iter().enumerate() {
        s.backup_put(owner, lookup, generation, i as u32, chunks.len() as u32, d.clone()).await?;
    }
    Ok(())
}

async fn get(s: &Store, lookup: &str) -> Option<Vec<Vec<u8>>> {
    let (parts, first) = s.backup_get(lookup, 0).await.unwrap()?;
    let mut out = vec![first];
    for i in 1..parts {
        out.push(s.backup_get(lookup, i).await.unwrap().unwrap().1);
    }
    Some(out)
}

fn assert_disco_falhou<T: std::fmt::Debug>(r: Result<T, StoreError>) {
    assert!(matches!(r, Err(StoreError::BackupDisk(_))), "{r:?}");
}

fn envelhecer(path: &Path, dias: u64) {
    let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(dias * DAY)).unwrap();
}

fn idade_em_dias(path: &Path) -> u64 {
    std::fs::metadata(path).unwrap().modified().unwrap().elapsed().unwrap_or_default().as_secs() / DAY
}

#[tokio::test]
async fn o_conteudo_vai_para_o_disco_e_o_redis_so_fica_com_o_indice() {
    let l = Lugar::new("onde");
    let s = l.store();
    let (owner, lookup) = (id('1'), id('2'));
    let chunks = vec![vec![1u8; 40_000], vec![2u8; 100]];
    put(&s, &owner, &lookup, G1, &chunks).await.unwrap();

    assert!(l.file(&lookup).is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(l.file(&lookup)).unwrap().permissions().mode() & 0o777, 0o600);
    }
    for i in 0..2 {
        assert!(!l.existe(format!("bkpart:{lookup}:{G1}:{i}")).await, "parte {i} ainda na RAM");
    }
    let ttl = l.ttl(format!("bkhead:{lookup}")).await;
    assert!(ttl > 89 * DAY as i64 && ttl <= BACKUP_TTL_SECONDS, "{ttl}");
    assert_eq!(get(&s, &lookup).await, Some(chunks));
    assert_eq!(s.backup_get(&lookup, 2).await.unwrap(), None);
    assert_eq!(get(&s, &id('3')).await, None);

    // Um envio a meio nao toca na copia publicada; completo, substitui-a.
    s.backup_put(&owner, &lookup, G2, 0, 2, vec![3; 10]).await.unwrap();
    assert_eq!(get(&s, &lookup).await.unwrap()[1], vec![2u8; 100]);
    s.backup_put(&owner, &lookup, G2, 1, 2, vec![4; 10]).await.unwrap();
    assert_eq!(get(&s, &lookup).await, Some(vec![vec![3; 10], vec![4; 10]]));

    // So o dono, como antes.
    let r = put(&s, &id('9'), &lookup, G1, &[vec![6; 4]]).await;
    assert!(matches!(r, Err(StoreError::BackupNotOwner)), "{r:?}");
    s.delete_backup(&owner).await.unwrap();
}

#[tokio::test]
async fn copia_antiga_no_redis_le_se_na_mesma_e_passa_para_o_disco_quando_a_app_abre() {
    let l = Lugar::new("antiga");
    let (owner, lookup) = (id('1'), id('2'));
    let chunks = vec![vec![1u8; 100], vec![2u8; 50]];
    put(&l.store_so_redis(), &owner, &lookup, G1, &chunks).await.unwrap();

    // Relay atualizado: le-se do Redis, ainda sem ficheiro.
    let s = l.store();
    assert_eq!(get(&s, &lookup).await, Some(chunks.clone()));
    assert!(!l.file(&lookup).exists());

    // A app abre: passa para o disco e sai da RAM.
    s.renew_backup(&owner).await.unwrap();
    assert!(l.file(&lookup).is_file());
    for i in 0..2 {
        assert!(!l.existe(format!("bkpart:{lookup}:{G1}:{i}")).await);
    }
    assert_eq!(get(&s, &lookup).await, Some(chunks.clone()));

    // Aberturas seguintes: so a data do ficheiro (e o prazo do indice).
    envelhecer(&l.file(&lookup), 40);
    s.renew_backup(&owner).await.unwrap();
    assert_eq!(idade_em_dias(&l.file(&lookup)), 0);

    // Um reenvio que ficou a meio, com a mesma geracao, nao estraga nada.
    s.backup_put(&owner, &lookup, G1, 0, 2, chunks[0].clone()).await.unwrap();
    s.renew_backup(&owner).await.unwrap();
    assert_eq!(get(&s, &lookup).await, Some(chunks));
    s.delete_backup(&owner).await.unwrap();
}

#[tokio::test]
async fn disco_cheio_nao_perde_nada() {
    let l = Lugar::new("cheio");
    let (owner, lookup) = (id('1'), id('2'));
    let s = l.store();
    let cheio = l.store_com_minimo_livre(u64::MAX);
    put(&s, &owner, &lookup, G1, &[vec![1; 100]]).await.unwrap();

    // Copia nova: recusada (a app nao a da como guardada) e fica a anterior.
    assert_disco_falhou(put(&cheio, &owner, &lookup, G2, &[vec![2; 100]]).await);
    assert_eq!(get(&s, &lookup).await, Some(vec![vec![1; 100]]));
    s.delete_backup(&owner).await.unwrap();

    // Copia antiga no Redis que nao se consegue mover: continua no Redis.
    put(&l.store_so_redis(), &owner, &lookup, G1, &[vec![3; 100]]).await.unwrap();
    assert_disco_falhou(cheio.renew_backup(&owner).await);
    assert!(l.existe(format!("bkpart:{lookup}:{G1}:0")).await);
    assert!(l.ttl(format!("bkpart:{lookup}:{G1}:0")).await > 89 * DAY as i64, "prazo renovado");
    assert_eq!(get(&cheio, &lookup).await, Some(vec![vec![3; 100]]));
    cheio.delete_backup(&owner).await.unwrap();
}

#[tokio::test]
async fn dados_estragados_ou_em_falta_dao_erro_e_nunca_nao_ha_copia() {
    let l = Lugar::new("estragado");
    let s = l.store();
    let (owner, lookup) = (id('1'), id('2'));
    put(&s, &owner, &lookup, G1, &[vec![1; 100]]).await.unwrap();

    std::fs::write(l.file(&lookup), b"lixo").unwrap();
    assert_disco_falhou(s.backup_get(&lookup, 0).await);
    std::fs::remove_file(l.file(&lookup)).unwrap();
    assert_disco_falhou(s.backup_get(&lookup, 0).await);
    // Pasta das copias que deixou de servir (o relay arrancou sem ela).
    assert_disco_falhou(l.store_so_redis().backup_get(&lookup, 0).await);

    // Nao se consegue escrever nem apagar: o indice fica, para tentar outra vez.
    std::fs::create_dir(l.file(&lookup)).unwrap();
    assert_disco_falhou(put(&s, &owner, &lookup, G2, &[vec![2; 100]]).await);
    assert_disco_falhou(s.delete_backup(&owner).await);
    assert!(l.existe(format!("bkhead:{lookup}")).await);
    assert!(l.existe(format!("bkof:{owner}")).await);
    std::fs::remove_dir(l.file(&lookup)).unwrap();
    s.delete_backup(&owner).await.unwrap();
    assert_eq!(get(&s, &lookup).await, None);
    // Sem temporarios esquecidos pelas escritas que falharam.
    assert_eq!(std::fs::read_dir(l.dir.join(&lookup[..2])).unwrap().count(), 0);
}

#[tokio::test]
async fn palavras_novas_apagar_e_conta_apagada_tiram_o_ficheiro() {
    let l = Lugar::new("apagar");
    let s = l.store();
    let (owner, velho, novo) = (id('1'), id('2'), id('3'));
    put(&s, &owner, &velho, G1, &[vec![1; 10]]).await.unwrap();
    put(&s, &owner, &novo, G2, &[vec![2; 10]]).await.unwrap();
    assert!(!l.file(&velho).exists(), "ficheiro das palavras antigas");
    assert_eq!(get(&s, &velho).await, None);
    assert_eq!(get(&s, &novo).await, Some(vec![vec![2; 10]]));
    s.delete_backup(&owner).await.unwrap();
    assert!(!l.file(&novo).exists());
    assert_eq!(get(&s, &novo).await, None);
}

#[tokio::test]
async fn banida_o_ficheiro_dura_30_dias_e_volta_aos_90() {
    let l = Lugar::new("banida");
    let s = l.store();
    let (owner, lookup) = (id('1'), id('2'));
    put(&s, &owner, &lookup, G1, &[vec![1; 10]]).await.unwrap();
    s.ban(&owner).await.unwrap();
    s.expire_backup_of_banned(&owner).await.unwrap();
    assert!(l.ttl(format!("bkhead:{lookup}")).await <= 30 * DAY as i64);
    assert_eq!(idade_em_dias(&l.file(&lookup)), 60);
    assert_eq!(get(&s, &lookup).await, Some(vec![vec![1; 10]]));
    s.unban(&owner).await.unwrap();
    s.renew_backup_of(&owner, &lookup).await.unwrap();
    assert!(l.ttl(format!("bkhead:{lookup}")).await > 30 * DAY as i64);
    assert_eq!(idade_em_dias(&l.file(&lookup)), 0);
    s.delete_backup(&owner).await.unwrap();
}

#[tokio::test]
async fn a_limpeza_so_apaga_o_que_esta_parado_ha_90_dias_e_sem_indice() {
    let l = Lugar::new("limpeza");
    let s = l.store();
    let (com_indice, sem_indice, recente) = (id('a'), id('b'), id('c'));
    for (owner, lookup) in [(id('1'), &com_indice), (id('2'), &sem_indice), (id('3'), &recente)] {
        put(&s, &owner, lookup, G1, &[vec![1; 10]]).await.unwrap();
    }
    envelhecer(&l.file(&com_indice), 100);
    envelhecer(&l.file(&sem_indice), 100);
    l.esquecer_indice(&sem_indice).await;
    // Sem indice mas recente (ex.: Redis esvaziado): fica.
    l.esquecer_indice(&recente).await;
    let pasta = l.dir.join("aa");
    let (abandonado, a_escrever, alheio) = (pasta.join("x.tmp"), pasta.join("y.tmp"), pasta.join("LEIA-ME"));
    for f in [&abandonado, &a_escrever, &alheio] {
        std::fs::write(f, b"x").unwrap();
    }
    envelhecer(&abandonado, 1);
    envelhecer(&alheio, 400);

    assert_eq!(s.sweep_backups().await.unwrap(), 2);
    assert!(l.file(&com_indice).exists());
    assert!(!l.file(&sem_indice).exists());
    assert!(l.file(&recente).exists());
    assert!(!abandonado.exists());
    assert!(a_escrever.exists() && alheio.exists());
    assert_eq!(l.store_so_redis().sweep_backups().await.unwrap(), 0);
}

/// A copia antiga a ser movida e uma nova a chegar ao mesmo tempo: ganha sempre a nova.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mover_a_antiga_nunca_passa_por_cima_de_uma_nova() {
    let l = Lugar::new("corrida");
    let antigo = l.store_so_redis();
    let s = std::sync::Arc::new(l.store());
    let (owner, lookup) = (id('1'), id('2'));
    for round in 0..60 {
        antigo.delete_backup(&owner).await.unwrap();
        let _ = std::fs::remove_file(l.file(&lookup));
        put(&antigo, &owner, &lookup, G1, &[vec![1; 100]]).await.unwrap();
        let (a, b) = (s.clone(), s.clone());
        let (o, l1) = (owner.clone(), lookup.clone());
        let mover = tokio::spawn(async move { a.renew_backup(&o).await });
        let (o, l2) = (owner.clone(), l1);
        let nova = tokio::spawn(async move { put(&b, &o, &l2, G2, &[vec![2; 100]]).await });
        mover.await.unwrap().unwrap();
        nova.await.unwrap().unwrap();
        assert_eq!(get(&s, &lookup).await, Some(vec![vec![2; 100]]), "volta {round}");
    }
    s.delete_backup(&owner).await.unwrap();
}
