//! O relay funciona com o Redis de producao: utilizador `qt-relay` com a lista
//! FECHADA de comandos de `deploy/redis/qt-redis.conf`. Os outros testes usam um
//! Redis sem restricoes, por isso nao apanhavam um comando novo esquecido na lista
//! (foi o que aconteceu com os anexos: "erro interno" so no servidor).
//!
//! Arranca um `redis-server` proprio numa porta livre com a linha `user qt-relay`
//! tirada do ficheiro de deploy. Sem `redis-server` instalado, o teste e saltado.

use base64::Engine;
use qt_server::{BlobConfig, Store};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Redis(Child);

impl Drop for Redis {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn start_redis_with_production_acl(password: &str) -> Option<(Redis, u16)> {
    if Command::new("redis-server").arg("--version").stdout(Stdio::null()).status().is_err() {
        eprintln!("redis-server nao instalado: teste saltado");
        return None;
    }
    let deploy = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../deploy/redis/qt-redis.conf")).unwrap();
    let acl = deploy.lines().find(|l| l.starts_with("user qt-relay ")).expect("linha do utilizador qt-relay");
    let port = free_port();
    let conf = std::env::temp_dir().join(format!("qt-acl-{}-{port}.conf", std::process::id()));
    std::fs::write(
        &conf,
        format!(
            "port {port}\nbind 127.0.0.1\nsave \"\"\nappendonly no\nuser default off\n{}\n",
            acl.replace("SUBSTITUI_POR_PASSWORD_FORTE", password)
        ),
    )
    .unwrap();
    let redis = Redis(Command::new("redis-server").arg(&conf).stdout(Stdio::null()).spawn().unwrap());
    for _ in 0..50 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Some((redis, port));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("o redis-server de teste nao arrancou"); // o Drop de `redis` termina o processo
}

#[tokio::test]
async fn relay_completo_com_a_lista_de_comandos_de_producao() {
    let pw = "ab12".repeat(16);
    let Some((_redis, port)) = start_redis_with_production_acl(&pw) else { return };
    let dir = std::env::temp_dir().join(format!("qt-acl-anexos-{}-{port}", std::process::id()));
    // Store::new usa o prefixo "default", o unico que o ACL deixa usar.
    let mut store = Store::new(&format!("redis://qt-relay:{pw}@127.0.0.1:{port}")).unwrap();
    let mut cfg = BlobConfig::new(&dir);
    cfg.min_free_bytes = 0;
    cfg.max_user_bytes_per_day = 1000;
    store.enable_blobs(cfg).unwrap();
    let alice = "a".repeat(64);

    // Mensagens e bundles (o que ja existia).
    store.register_bundle(alice.clone(), vec![1, 2, 3]).await.unwrap();
    assert_eq!(store.get_bundle(&alice).await.unwrap(), Some(vec![1, 2, 3]));
    store.enqueue_message(&alice, alice.clone(), vec![], vec![9; 10]).await.unwrap();
    let q = store.peek_messages(&alice, 40_000).await.unwrap();
    assert_eq!(q.len(), 1);
    assert_eq!(store.ack_messages(&alice, &[q[0].id]).await.unwrap(), 1);

    // One-time prekeys (0.9.3): DEL + RPUSH + EXPIRE numa transacao (MULTI/EXEC), LPOP e LLEN.
    store.replace_one_time_prekeys(&alice, &[vec![1; 36], vec![2; 36]]).await.unwrap();
    assert_eq!(store.count_one_time_prekeys(&alice).await.unwrap(), 2);
    assert_eq!(store.pop_one_time_prekey(&alice).await.unwrap(), Some(vec![1; 36]));
    store.replace_one_time_prekeys(&alice, &[vec![3; 36]]).await.unwrap();
    assert_eq!(store.pop_one_time_prekey(&alice).await.unwrap(), Some(vec![3; 36]));
    assert_eq!(store.pop_one_time_prekey(&alice).await.unwrap(), None);

    // Anexos: todos os comandos novos (HMSET, HSET, HGET, HGETALL, DEL, INCRBY, DECRBY).
    let (id, token) = store.blob_create(&alice, 5).await.unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode([7u8; 5]);
    assert!(store.blob_put(&id, &token, 0, &b64).await.unwrap().complete);
    assert!(store.blob_info(&id).await.unwrap().complete);
    assert_eq!(store.blob_get(&id, 0).await.unwrap(), (5, vec![7u8; 5]));
    store.sweep_blobs().await.unwrap();
    store.blob_delete(&id).await.unwrap();
    assert!(matches!(store.blob_info(&id).await, Err(qt_server::blobs::BlobError::NotFound)));
    // Quota diaria ultrapassada: recusado, e os bytes reservados voltam ao contador
    // (DECRBY). Se nao voltassem, o pedido pequeno a seguir tambem seria recusado.
    assert!(matches!(store.blob_create(&alice, 2000).await, Err(qt_server::blobs::BlobError::Quota)));
    assert!(store.blob_create(&alice, 5).await.is_ok());

    // Apagar a conta (DeleteAccount): so comandos da lista (DEL, SET, EXPIRE, GET).
    store.enqueue_message(&alice, alice.clone(), vec![], vec![1; 4]).await.unwrap();
    store.delete_account(&alice).await.unwrap();
    assert_eq!(store.get_bundle(&alice).await.unwrap(), None);
    assert!(store.peek_messages(&alice, 40_000).await.unwrap().is_empty());
    assert!(matches!(
        store.enqueue_message(&alice, alice.clone(), vec![], vec![1; 4]).await,
        Err(qt_server::store::StoreError::RecipientDeleted)
    ));
    // Publicar um bundle de novo desfaz a eliminacao.
    store.register_bundle(alice.clone(), vec![4]).await.unwrap();
    store.enqueue_message(&alice, alice.clone(), vec![], vec![1; 4]).await.unwrap();

    // Denuncias e banimentos (operador).
    let id = store.submit_report(&alice, vec![1; 100]).await.unwrap();
    let (reports, more) = store.list_reports(0, 40_000).await.unwrap();
    assert_eq!((reports.len(), more, reports[0].id), (1, false, id));
    store.delete_report(id).await.unwrap();
    assert!(store.list_reports(0, 40_000).await.unwrap().0.is_empty());
    store.ban(&alice).await.unwrap();
    assert!(store.is_banned(&alice).await.unwrap());
    assert_eq!(store.list_bans().await.unwrap(), vec![alice.clone()]);
    store.unban(&alice).await.unwrap();
    assert!(!store.is_banned(&alice).await.unwrap());

    // Copia de recuperacao.
    let lookup = "ab".repeat(32);
    store.backup_put(&alice, &lookup, "0123456789abcdef", 0, 1, vec![5; 10]).await.unwrap();
    assert_eq!(store.backup_get(&lookup, 0).await.unwrap(), Some((1, vec![5; 10])));
    store.renew_backup(&alice).await.unwrap();
    store.delete_backup(&alice).await.unwrap();
    assert_eq!(store.backup_get(&lookup, 0).await.unwrap(), None);
    let _ = std::fs::remove_dir_all(dir);
}
