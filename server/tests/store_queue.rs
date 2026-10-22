//! Fila de mensagens no Redis: confirmar (ack) apaga exatamente as mensagens
//! confirmadas, estejam onde estiverem na fila, mesmo com confirmacoes repetidas
//! ou ao mesmo tempo. Precisa de um Redis em 127.0.0.1:6379.

use qt_server::Store;
use std::sync::Arc;

fn store(tag: &str) -> Arc<Store> {
    let mut b = [0u8; 4];
    use std::io::Read;
    std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut b).unwrap();
    Arc::new(Store::with_prefix("redis://127.0.0.1:6379", &format!("t-fila-{tag}-{}-{}", std::process::id(), u32::from_le_bytes(b))).unwrap())
}

/// So contas com bundle trocam mensagens (o relay recusa as outras): aqui cada
/// utilizador escreve para si proprio.
async fn reg(s: &Store, user: &str) {
    s.register_bundle(user.into(), vec![1]).await.unwrap();
}

async fn fill(s: &Store, user: &str, n: u8) -> Vec<u64> {
    reg(s, user).await;
    for i in 0..n {
        s.enqueue_message(user, user.into(), vec![], vec![i; 10]).await.unwrap();
    }
    s.peek_messages(user, 1_000_000).await.unwrap().iter().map(|m| m.id).collect()
}

async fn remaining(s: &Store, user: &str) -> Vec<u8> {
    s.peek_messages(user, 1_000_000).await.unwrap().iter().map(|m| m.ciphertext[0]).collect()
}

#[tokio::test]
async fn ack_apaga_so_as_confirmadas_em_qualquer_posicao() {
    let s = store("pos");
    let u = "a".repeat(64);
    let ids = fill(&s, &u, 10).await;
    assert_eq!(ids.len(), 10);
    // As primeiras (caso normal).
    assert_eq!(s.ack_messages(&u, &ids[..3]).await.unwrap(), 3);
    assert_eq!(remaining(&s, &u).await, vec![3, 4, 5, 6, 7, 8, 9]);
    // Do meio e do fim, fora de ordem, com repetidos e ids que nao existem.
    assert_eq!(s.ack_messages(&u, &[ids[9], ids[5], ids[5], 999_999_999, ids[0]]).await.unwrap(), 2);
    assert_eq!(remaining(&s, &u).await, vec![3, 4, 6, 7, 8]);
    // Repetir uma confirmacao nao apaga mais nada.
    assert_eq!(s.ack_messages(&u, &ids[..3]).await.unwrap(), 0);
    assert_eq!(s.ack_messages(&u, &[]).await.unwrap(), 0);
    assert_eq!(remaining(&s, &u).await, vec![3, 4, 6, 7, 8]);
    // Uma pessoa nao apaga a fila de outra (os ids sao globais).
    let v = "b".repeat(64);
    fill(&s, &v, 2).await;
    assert_eq!(s.ack_messages(&v, &[ids[3], ids[4]]).await.unwrap(), 0);
    assert_eq!(remaining(&s, &u).await, vec![3, 4, 6, 7, 8]);
}

#[tokio::test]
async fn confirmacoes_ao_mesmo_tempo_nunca_apagam_mensagens_por_confirmar() {
    let s = store("conc");
    let u = "c".repeat(64);
    let ids = fill(&s, &u, 60).await;
    // 8 confirmacoes iguais das primeiras 20, ao mesmo tempo (ex.: ligacao antiga + nova).
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let (s, u, first) = (s.clone(), u.clone(), ids[..20].to_vec());
        tasks.push(tokio::spawn(async move { s.ack_messages(&u, &first).await.unwrap() }));
    }
    let mut total = 0;
    for t in tasks {
        total += t.await.unwrap();
    }
    assert_eq!(total, 20, "cada mensagem e apagada uma so vez");
    assert_eq!(remaining(&s, &u).await, (20u8..60).collect::<Vec<_>>());
}

#[tokio::test]
async fn fila_cheia_recusa_e_ids_sao_unicos() {
    let s = store("cheia");
    let u = "d".repeat(64);
    reg(&s, &u).await;
    for i in 0..1000u32 {
        s.enqueue_message(&u, u.clone(), vec![], i.to_be_bytes().to_vec()).await.unwrap();
    }
    assert!(matches!(
        s.enqueue_message(&u, u.clone(), vec![], vec![1]).await,
        Err(qt_server::store::StoreError::QueueFull)
    ));
    let mut all = Vec::new();
    loop {
        let page = s.peek_messages(&u, 40_000).await.unwrap();
        if page.is_empty() {
            break;
        }
        let ids: Vec<u64> = page.iter().map(|m| m.id).collect();
        all.extend(ids.iter().copied());
        assert_eq!(s.ack_messages(&u, &ids).await.unwrap(), ids.len());
    }
    assert_eq!(all.len(), 1000);
    let unique: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(unique.len(), 1000, "ids repetidos");
    assert!(all.windows(2).all(|w| w[0] < w[1]), "ordem de chegada mantida");
}

/// A medicao do tempo de entrega (latency.rs) so conta ids que sairam mesmo da
/// fila: uma confirmacao repetida, ou de um id que nao esta na fila, nao conta.
#[tokio::test]
async fn ack_devolve_so_os_ids_que_sairam_da_fila() {
    let s = store("ids");
    let u = "c".repeat(64);
    reg(&s, &u).await;
    let id = s.enqueue_message(&u, u.clone(), vec![], vec![1]).await.unwrap();
    assert_eq!(s.ack_messages_ids(&u, &[id, id + 1_000_000]).await.unwrap(), vec![id]);
    assert!(s.ack_messages_ids(&u, &[id]).await.unwrap().is_empty());
}

/// Regressao (22/10/2026): chaves descartaveis, sem bundle nem prova de trabalho,
/// enchiam filas de IDs inventados ate o Redis ficar sem memoria. So contas com
/// bundle escrevem, e so para contas com bundle; o que e recusado nao fica guardado.
#[tokio::test]
async fn so_contas_registadas_trocam_mensagens() {
    use qt_server::store::StoreError;
    let s = store("registo");
    let (a, b, x) = ("e".repeat(64), "f".repeat(64), "1".repeat(64));
    // Remetente sem bundle: recusado (mesmo para um destinatario que existe).
    reg(&s, &b).await;
    assert!(matches!(s.enqueue_message(&a, b.clone(), vec![], vec![1]).await, Err(StoreError::SenderNotRegistered)));
    assert!(s.peek_messages(&b, 40_000).await.unwrap().is_empty());
    // Remetente registado, destinatario que nunca existiu: recusado e nada guardado.
    reg(&s, &a).await;
    assert!(matches!(s.enqueue_message(&a, x.clone(), vec![], vec![1]).await, Err(StoreError::RecipientUnknown)));
    assert!(s.peek_messages(&x, 40_000).await.unwrap().is_empty());
    // Os dois registados: entregue.
    s.enqueue_message(&a, b.clone(), vec![], vec![2]).await.unwrap();
    assert_eq!(remaining(&s, &b).await, vec![2]);
    // Quem apagou a conta continua a ter a resposta propria (a app mostra o aviso).
    s.delete_account(&b).await.unwrap();
    assert!(matches!(s.enqueue_message(&a, b.clone(), vec![], vec![3]).await, Err(StoreError::RecipientDeleted)));
    assert!(s.is_registered(&a).await.unwrap());
    assert!(!s.is_registered(&b).await.unwrap());
}
