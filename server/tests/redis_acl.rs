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
    start_redis_with_acl_lines(&[("user qt-relay ", "SUBSTITUI_POR_PASSWORD_FORTE", password)])
}

/// Redis de teste com as linhas `user ...` do ficheiro de deploy (prefixo, marcador, password).
fn start_redis_with_acl_lines(users: &[(&str, &str, &str)]) -> Option<(Redis, u16)> {
    if Command::new("redis-server").arg("--version").stdout(Stdio::null()).status().is_err() {
        eprintln!("redis-server nao instalado: teste saltado");
        return None;
    }
    let deploy = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../deploy/redis/qt-redis.conf")).unwrap();
    let mut acl = String::new();
    for (prefix, marker, password) in users {
        let line = deploy.lines().find(|l| l.starts_with(prefix)).expect("linha do utilizador no qt-redis.conf");
        acl.push_str(&line.replace(marker, password));
        acl.push('\n');
    }
    let port = free_port();
    let conf = std::env::temp_dir().join(format!("qt-acl-{}-{port}.conf", std::process::id()));
    std::fs::write(&conf, format!("port {port}\nbind 127.0.0.1\nsave \"\"\nappendonly no\nuser default off\n{acl}")).unwrap();
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

    // 0024 (EXISTS e TTL na lista): estado ao autenticar, envio num so pedido, recusa
    // desfeita com o prazo da fila reposto, confirmacao com a pagina guardada, registo numa
    // transacao (MULTI/EXEC) e premium num so pedido.
    assert_eq!(store.auth_state(&alice).await.unwrap(), (false, true));
    assert!(store.is_registered(&alice).await.unwrap());
    store.enqueue(qt_server::store::Sender::Registered, alice.clone(), vec![], vec![3; 10]).await.unwrap();
    let page = store.peek_messages_raw(&alice, 40_000).await.unwrap();
    let mut known: Vec<(u64, Vec<u8>)> = page.into_iter().map(|(m, r)| (m.id, r)).collect();
    let first = known[0].0;
    assert_eq!(store.ack_known(&alice, &[first], &mut known).await.unwrap(), vec![first]);
    let bob = "b".repeat(64);
    store.register_bundle(bob.clone(), vec![1]).await.unwrap();
    store.enqueue_message(&alice, bob.clone(), vec![], vec![1]).await.unwrap();
    store.fed_largar(&bob).await.unwrap();
    assert!(matches!(
        store.enqueue_message(&alice, bob.clone(), vec![], vec![1]).await,
        Err(qt_server::store::StoreError::RecipientUnknown)
    ));
    assert_eq!(store.register_full(&alice, vec![1, 2, 3], Some(&[vec![4; 36]])).await.unwrap(), None);
    assert!(store.premium_allows(&alice).await.unwrap());

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

    // Premium.
    store.set_premium(&alice, 4_102_444_800_000).await.unwrap();
    assert_eq!(store.premium_until(&alice).await.unwrap(), 4_102_444_800_000);
    assert_eq!(store.list_premium().await.unwrap().len(), 1);
    store.set_premium(&alice, 0).await.unwrap();
    assert_eq!(store.premium_until(&alice).await.unwrap(), 0);

    // Comunidade (0.9.52): envios como as denuncias e a lista com GET, SET, EXPIRE e MULTI/EXEC.
    let id = store.submit_community(&alice, vec![2; 100]).await.unwrap();
    let (subs, more) = store.list_community(0, 40_000).await.unwrap();
    assert_eq!((subs.len(), more, subs[0].0), (1, false, id));
    store.delete_community(id).await.unwrap();
    assert!(store.list_community(0, 40_000).await.unwrap().0.is_empty());
    let h = "cd".repeat(32);
    assert_eq!(store.community_keep(&[h.clone()]).await.unwrap(), vec![h.clone()]);
    store.community_put_photo(&h, vec![8; 50]).await.unwrap();
    assert!(store.community_keep(&[h.clone()]).await.unwrap().is_empty());
    store.community_publish(7, vec![9; 40]).await.unwrap();
    assert_eq!(store.community_version().await.unwrap(), 7);
    assert_eq!(store.community_index().await.unwrap(), Some((7, vec![9; 40])));
    assert_eq!(store.community_photo(&h).await.unwrap(), Some(vec![8; 50]));
    let _ = std::fs::remove_dir_all(dir);
}

/// Federacao (fase 1): o estado das contas, a marca "por verificar" e largar uma conta
/// so usam comandos da lista (GET, SET, EXPIRE, DEL, RPUSH, LRANGE, LREM).
#[tokio::test]
async fn federacao_com_a_lista_de_comandos_de_producao() {
    use qt_server::federation::Estado;
    let pw = "cd34".repeat(16);
    let Some((_redis, port)) = start_redis_with_production_acl(&pw) else { return };
    let store = Store::new(&format!("redis://qt-relay:{pw}@127.0.0.1:{port}")).unwrap();
    let id = "b".repeat(64);
    assert_eq!(store.fed_estado(&id).await.unwrap(), Estado::NaoTenho);
    store.register_bundle(id.clone(), vec![1]).await.unwrap();
    store.fed_marcar_por_verificar(&id).await.unwrap();
    assert_eq!(store.fed_estado(&id).await.unwrap(), Estado::TenhoPorVerificar);
    assert_eq!(store.fed_por_verificar(10).await.unwrap(), vec![id.clone()]);
    store.fed_verificado(&id).await.unwrap();
    assert_eq!(store.fed_estado(&id).await.unwrap(), Estado::Tenho);
    store.set_push_token(&id, Some("tok-1")).await.unwrap();
    store.fed_marcar_por_verificar(&id).await.unwrap();
    store.fed_largar(&id).await.unwrap();
    assert_eq!(store.fed_estado(&id).await.unwrap(), Estado::NaoTenho);
    assert_eq!(store.get_push_token(&id).await.unwrap(), None);
    assert!(store.fed_por_verificar(10).await.unwrap().is_empty());
    store.delete_account(&id).await.unwrap();
    assert_eq!(store.fed_estado(&id).await.unwrap(), Estado::Apagada);
    store.ban(&id).await.unwrap();
    assert_eq!(store.fed_estado(&id).await.unwrap(), Estado::Banida);
}
