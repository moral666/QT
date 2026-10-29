//! Menos idas ao Redis (0024): envio num so pedido (com desfazer), ids por blocos,
//! confirmacao com a pagina guardada, registo numa transacao, estado ao autenticar e
//! premium num so pedido. Precisa de um Redis em 127.0.0.1:6379.

use qt_server::store::{Sender, StoreError};
use qt_server::Store;
use std::sync::Arc;

const URL: &str = "redis://127.0.0.1:6379";

fn prefix(tag: &str) -> String {
    let mut b = [0u8; 4];
    use std::io::Read;
    std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut b).unwrap();
    format!("t-pipe-{tag}-{}-{}", std::process::id(), u32::from_le_bytes(b))
}

fn store(p: &str) -> Arc<Store> {
    Arc::new(Store::with_prefix(URL, p).unwrap())
}

async fn raw() -> redis::aio::MultiplexedConnection {
    redis::Client::open(URL).unwrap().get_multiplexed_async_connection().await.unwrap()
}

async fn exists(p: &str, key: &str) -> bool {
    redis::cmd("EXISTS").arg(format!("{p}:{key}")).query_async(&mut raw().await).await.unwrap()
}

async fn ttl(p: &str, key: &str) -> i64 {
    redis::cmd("TTL").arg(format!("{p}:{key}")).query_async(&mut raw().await).await.unwrap()
}

async fn llen(p: &str, key: &str) -> usize {
    redis::cmd("LLEN").arg(format!("{p}:{key}")).query_async(&mut raw().await).await.unwrap()
}

fn user(c: char) -> String {
    c.to_string().repeat(64)
}

#[tokio::test]
async fn ids_por_blocos_sao_unicos_e_crescem_depois_de_reiniciar() {
    let p = prefix("ids");
    let a = user('a');
    let s1 = store(&p);
    s1.register_bundle(a.clone(), vec![1]).await.unwrap();
    // Muitos envios ao mesmo tempo: todos com ids diferentes.
    let mut tasks = Vec::new();
    for i in 0..200u32 {
        let (s, a) = (s1.clone(), a.clone());
        tasks.push(tokio::spawn(async move { s.enqueue_message(&a, a.clone(), vec![], i.to_be_bytes().to_vec()).await.unwrap() }));
    }
    let mut ids = Vec::new();
    for t in tasks {
        ids.push(t.await.unwrap());
    }
    let max_antes = *ids.iter().max().unwrap();
    // "Reinicio": outro Store com o mesmo prefixo reserva um bloco novo, acima de todos.
    let s2 = store(&p);
    let depois = s2.enqueue_message(&a, a.clone(), vec![], vec![1]).await.unwrap();
    assert!(depois > max_antes, "{depois} > {max_antes}");
    ids.push(depois);
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "ids repetidos");
}

#[tokio::test]
async fn envio_recusado_nao_deixa_chaves_para_tras() {
    let p = prefix("recusa");
    let s = store(&p);
    let (a, x) = (user('a'), user('x'));
    s.register_bundle(a.clone(), vec![1]).await.unwrap();
    // Destinatario desconhecido.
    assert!(matches!(s.enqueue_message(&a, x.clone(), vec![], vec![1]).await, Err(StoreError::RecipientUnknown)));
    assert!(!exists(&p, &format!("queue:{x}")).await);
    // Destinatario que apagou a conta.
    let d = user('d');
    s.register_bundle(d.clone(), vec![1]).await.unwrap();
    s.delete_account(&d).await.unwrap();
    assert!(matches!(s.enqueue_message(&a, d.clone(), vec![], vec![1]).await, Err(StoreError::RecipientDeleted)));
    assert!(!exists(&p, &format!("queue:{d}")).await);
    // Remetente sem conta (a verificar no mesmo pedido).
    let n = user('n');
    assert!(matches!(s.enqueue(Sender::Check(&n), a.clone(), vec![], vec![1]).await, Err(StoreError::SenderNotRegistered)));
    assert!(!exists(&p, &format!("queue:{a}")).await);
    // Vinda de outro relay para um desconhecido: o mesmo.
    assert!(matches!(s.enqueue_from_peer(x.clone(), vec![], vec![1]).await, Err(StoreError::RecipientUnknown)));
    assert!(!exists(&p, &format!("queue:{x}")).await);
}

#[tokio::test]
async fn recusa_repoe_o_prazo_de_uma_fila_que_ja_existia() {
    let p = prefix("prazo");
    let s = store(&p);
    let (a, b) = (user('a'), user('b'));
    s.register_bundle(a.clone(), vec![1]).await.unwrap();
    s.register_bundle(b.clone(), vec![1]).await.unwrap();
    s.enqueue_message(&a, b.clone(), vec![], vec![7]).await.unwrap();
    // A fila de b fica com pouco prazo e b deixa de ter bundle aqui (como depois de a
    // federacao a levar para outro relay): a fila velha tem de expirar na mesma.
    let _: () = redis::cmd("EXPIRE").arg(format!("{p}:queue:{b}")).arg(100).query_async(&mut raw().await).await.unwrap();
    s.fed_largar(&b).await.unwrap();
    for _ in 0..3 {
        assert!(matches!(s.enqueue_message(&a, b.clone(), vec![], vec![8]).await, Err(StoreError::RecipientUnknown)));
    }
    let t = ttl(&p, &format!("queue:{b}")).await;
    assert!((1..=100).contains(&t), "prazo reposto: {t}");
    assert_eq!(llen(&p, &format!("queue:{b}")).await, 1, "so a mensagem antiga");
}

#[tokio::test]
async fn fila_cheia_desfaz_sem_mexer_no_prazo() {
    let p = prefix("cheia");
    let s = store(&p);
    let a = user('a');
    s.register_bundle(a.clone(), vec![1]).await.unwrap();
    for i in 0..1000u32 {
        s.enqueue(Sender::Registered, a.clone(), vec![], i.to_be_bytes().to_vec()).await.unwrap();
    }
    let _: () = redis::cmd("EXPIRE").arg(format!("{p}:queue:{a}")).arg(100).query_async(&mut raw().await).await.unwrap();
    assert!(matches!(s.enqueue(Sender::Registered, a.clone(), vec![], vec![1]).await, Err(StoreError::QueueFull)));
    assert_eq!(llen(&p, &format!("queue:{a}")).await, 1000);
    assert!(ttl(&p, &format!("queue:{a}")).await <= 100);
}

#[tokio::test]
async fn remetente_ja_verificado_na_ligacao_nao_e_verificado_outra_vez() {
    let p = prefix("reg");
    let s = store(&p);
    let (a, sem) = (user('a'), user('s'));
    s.register_bundle(a.clone(), vec![1]).await.unwrap();
    // Sender::Registered: a ligacao ja viu o bundle; o pedido nao o verifica.
    s.enqueue(Sender::Registered, a.clone(), vec![], vec![1]).await.unwrap();
    assert!(matches!(s.enqueue(Sender::Check(&sem), a.clone(), vec![], vec![1]).await, Err(StoreError::SenderNotRegistered)));
    assert_eq!(s.peek_messages(&a, 1_000_000).await.unwrap().len(), 1);
}

#[tokio::test]
async fn confirmacao_com_a_pagina_guardada() {
    let p = prefix("ack");
    let s = store(&p);
    let a = user('a');
    s.register_bundle(a.clone(), vec![1]).await.unwrap();
    for i in 0..5u8 {
        s.enqueue_message(&a, a.clone(), vec![i], vec![i; 10]).await.unwrap();
    }
    let page = s.peek_messages_raw(&a, 1_000_000).await.unwrap();
    let ids: Vec<u64> = page.iter().map(|(m, _)| m.id).collect();
    let mut known: Vec<(u64, Vec<u8>)> = page.into_iter().map(|(m, r)| (m.id, r)).collect();
    // Outra ligacao da mesma conta com a mesma pagina.
    let mut known2 = known.clone();
    // Confirma 3 (uma repetida): saem da fila e da pagina guardada.
    let out = s.ack_known(&a, &[ids[0], ids[1], ids[1], ids[2]], &mut known).await.unwrap();
    assert_eq!(out, vec![ids[0], ids[1], ids[2]]);
    assert_eq!(known.len(), 2);
    // A outra ligacao confirma as mesmas: ja nao estao la, nada sai duas vezes.
    assert!(s.ack_known(&a, &ids[..3], &mut known2).await.unwrap().is_empty());
    // Um id que nao esta na pagina guardada vai pelo caminho de sempre.
    let mut vazio = Vec::new();
    assert_eq!(s.ack_known(&a, &[ids[3]], &mut vazio).await.unwrap(), vec![ids[3]]);
    let fica: Vec<u64> = s.peek_messages(&a, 1_000_000).await.unwrap().iter().map(|m| m.id).collect();
    assert_eq!(fica, vec![ids[4]]);
    // Tudo confirmado: a pagina guardada fica vazia e sem memoria (ids[3] ja tinha saido
    // da fila pela outra via, por isso so ids[4] conta).
    assert_eq!(s.ack_known(&a, &[ids[3], ids[4]], &mut known).await.unwrap(), vec![ids[4]]);
    assert!(known.is_empty() && known.capacity() == 0);
}

#[tokio::test]
async fn registo_numa_so_transacao() {
    let p = prefix("registo");
    let s = store(&p);
    let a = user('a');
    s.register_bundle(a.clone(), vec![1]).await.unwrap();
    s.delete_account(&a).await.unwrap();
    // Registar de novo: bundle, fim da marca de apagada e one-time prekeys trocadas.
    let otpks = vec![vec![1u8; 36], vec![2u8; 36]];
    assert_eq!(s.register_full(&a, vec![9, 9], Some(&otpks)).await.unwrap(), None);
    assert_eq!(s.get_bundle(&a).await.unwrap(), Some(vec![9, 9]));
    assert!(!exists(&p, &format!("deleted:{a}")).await);
    assert_eq!(s.count_one_time_prekeys(&a).await.unwrap(), 2);
    assert!(ttl(&p, &format!("bundle:{a}")).await > 0);
    // Sem one-time prekeys no pedido: as que estao ficam.
    s.register_full(&a, vec![9], None).await.unwrap();
    assert_eq!(s.count_one_time_prekeys(&a).await.unwrap(), 2);
    // Lista vazia: tira-as todas (como antes).
    s.register_full(&a, vec![9], Some(&[])).await.unwrap();
    assert_eq!(s.count_one_time_prekeys(&a).await.unwrap(), 0);
    // Com copia de recuperacao: o registo devolve o lookup para a renovar.
    let lookup = "ab".repeat(32);
    s.backup_put(&a, &lookup, "0123456789abcdef", 0, 1, vec![5; 10]).await.unwrap();
    assert_eq!(s.register_full(&a, vec![9], None).await.unwrap(), Some(lookup.clone()));
    s.renew_backup_of(&a, &lookup).await.unwrap();
    assert_eq!(s.backup_get(&lookup, 0).await.unwrap(), Some((1, vec![5; 10])));
}

#[tokio::test]
async fn estado_ao_autenticar_e_premium_num_so_pedido() {
    let p = prefix("auth");
    let s = store(&p);
    let (a, b) = (user('a'), user('b'));
    assert_eq!(s.auth_state(&a).await.unwrap(), (false, false));
    s.register_bundle(a.clone(), vec![1]).await.unwrap();
    assert_eq!(s.auth_state(&a).await.unwrap(), (false, true));
    assert!(s.is_registered(&a).await.unwrap());
    s.ban(&b).await.unwrap();
    assert_eq!(s.auth_state(&b).await.unwrap(), (true, false));
    // Premium: sem bloqueio passa sempre; com bloqueio so quem tem premium.
    assert!(s.premium_allows(&a).await.unwrap());
    s.set_premium_enforced(true).await.unwrap();
    assert!(!s.premium_allows(&a).await.unwrap());
    s.set_premium(&a, 4_102_444_800_000).await.unwrap();
    assert!(s.premium_allows(&a).await.unwrap());
}
