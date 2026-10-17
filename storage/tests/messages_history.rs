//! Historico de mensagens e outbox: inserir, deduplicar, estados, paginacao,
//! conversas, isolamento entre contactos e persistencia ao reabrir.

use qt_storage::messages::*;
use qt_storage::open_database_with_key;
use qt_storage::messages as hm;

fn db(name: &str) -> String {
    let p = std::env::temp_dir().join(format!("qt-hist-{}-{}.db", name, std::process::id()));
    let _ = std::fs::remove_file(&p);
    p.to_string_lossy().to_string()
}

const KEY: [u8; 32] = [7u8; 32];

#[test]
fn inserir_deduplicar_paginar_e_persistir() {
    let path = db("pag");
    {
        let c = open_database_with_key(&path, &KEY).unwrap();
        for i in 0..120u8 {
            let id = vec![i; 16];
            let out = i % 2 == 0;
            let st = if out { ST_SENT } else { ST_RECEIVED };
            assert!(insert_message(&c, &id, "alice", out, KIND_TEXT, &format!("m{i}"), i as i64, i as i64, st).unwrap().is_some());
        }
        // Duplicado (mesmo id, mesmo contacto) e ignorado.
        assert!(insert_message(&c, &[5u8; 16], "alice", false, KIND_TEXT, "dup", 0, 0, ST_RECEIVED).unwrap().is_none());
        // O mesmo id noutro contacto e outra mensagem.
        assert!(insert_message(&c, &[5u8; 16], "bob", false, KIND_TEXT, "b", 0, 0, ST_RECEIVED).unwrap().is_some());
    }
    // Reabrir: tudo continua la.
    let c = open_database_with_key(&path, &KEY).unwrap();
    let p1 = messages_page(&c, "alice", None, 50).unwrap();
    assert_eq!(p1.len(), 50);
    assert_eq!(p1[0].body, "m119", "primeira pagina comeca pela mais recente");
    let p2 = messages_page(&c, "alice", Some(p1.last().unwrap().seq), 50).unwrap();
    assert_eq!(p2[0].body, "m69");
    let p3 = messages_page(&c, "alice", Some(p2.last().unwrap().seq), 50).unwrap();
    assert_eq!(p3.len(), 20);
    assert_eq!(p3.last().unwrap().body, "m0");
    let after = messages_after(&c, "alice", p1[1].seq).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].body, "m119");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn estados_so_sobem_e_so_no_proprio_contacto() {
    let path = db("estados");
    let c = open_database_with_key(&path, &KEY).unwrap();
    let id = vec![1u8; 16];
    insert_message(&c, &id, "alice", true, KIND_TEXT, "ola", 1, 1, ST_PENDING).unwrap();
    mark_sent(&c, &id).unwrap();
    assert_eq!(get_message(&c, "alice", &id).unwrap().unwrap().status, ST_SENT);
    // Outro contacto nao consegue marcar as mensagens da alice.
    assert_eq!(raise_outgoing_status(&c, "mallory", std::slice::from_ref(&id), ST_READ).unwrap(), 0);
    assert_eq!(raise_outgoing_status(&c, "alice", std::slice::from_ref(&id), ST_READ).unwrap(), 1);
    // Um "entregue" atrasado nao desce um "visto".
    assert_eq!(raise_outgoing_status(&c, "alice", std::slice::from_ref(&id), ST_DELIVERED).unwrap(), 0);
    assert_eq!(get_message(&c, "alice", &id).unwrap().unwrap().status, ST_READ);
    // mark_sent depois de "visto" nao faz nada.
    mark_sent(&c, &id).unwrap();
    assert_eq!(get_message(&c, "alice", &id).unwrap().unwrap().status, ST_READ);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn conversas_por_ler_e_marcar_lidas() {
    let path = db("conv");
    let c = open_database_with_key(&path, &KEY).unwrap();
    insert_message(&c, &[1; 16], "alice", false, KIND_TEXT, "a1", 0, 0, ST_RECEIVED).unwrap();
    insert_message(&c, &[2; 16], "bob", false, KIND_TEXT, "b1", 0, 0, ST_RECEIVED).unwrap();
    insert_message(&c, &[3; 16], "alice", false, KIND_TEXT, "a2", 0, 0, ST_RECEIVED).unwrap();
    let conv = conversations(&c).unwrap();
    assert_eq!(conv.len(), 2);
    assert_eq!(conv[0].contact_id, "alice", "a mais recente primeiro");
    assert_eq!(conv[0].last.body, "a2");
    assert_eq!(conv[0].unread, 2);
    let ids = mark_incoming_read(&c, "alice").unwrap();
    assert_eq!(ids, vec![vec![1u8; 16], vec![3u8; 16]]);
    assert_eq!(conversations(&c).unwrap()[0].unread, 0);
    assert!(mark_incoming_read(&c, "alice").unwrap().is_empty(), "segunda vez nao ha nada por ler");
    delete_conversation(&c, "bob").unwrap();
    assert_eq!(conversations(&c).unwrap().len(), 1);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn outbox_ordem_e_remocao() {
    let path = db("outbox");
    let c = open_database_with_key(&path, &KEY).unwrap();
    outbox_put(&c, &[1; 16], "alice", OUTBOX_MESSAGE, b"x").unwrap();
    outbox_put(&c, &[2; 16], "bob", OUTBOX_RECEIPT, b"y").unwrap();
    let all = outbox_all(&c).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].outbox_id, vec![1u8; 16]);
    let e = outbox_take(&c, &[1; 16]).unwrap().unwrap();
    assert_eq!(e.blob, b"x");
    assert!(outbox_take(&c, &[1; 16]).unwrap().is_none());
    assert_eq!(outbox_all(&c).unwrap().len(), 1);
    let _ = std::fs::remove_file(&path);
}

/// Uma base criada por uma versao anterior (sem a coluna `sender`) ganha-a ao abrir,
/// sem perder as mensagens.
#[test]
fn base_antiga_ganha_a_coluna_sender() {
    let path = db("migra");
    {
        let c = open_database_with_key(&path, &KEY).unwrap();
        hm::insert_message(&c, &[1; 16], "aa", false, hm::KIND_TEXT, "antiga", 1, 1, hm::ST_RECEIVED).unwrap();
        // Simula a versao anterior: sem a coluna.
        c.execute_batch("ALTER TABLE messages DROP COLUMN sender").unwrap();
    }
    let c = open_database_with_key(&path, &KEY).unwrap();
    let m = hm::messages_page(&c, "aa", None, 10).unwrap();
    assert_eq!(m.len(), 1);
    assert_eq!((m[0].body.as_str(), m[0].sender.as_str()), ("antiga", ""));
    hm::insert_message_from(&c, &[2; 16], "g:01", false, hm::KIND_TEXT, "no grupo", 2, 2, hm::ST_RECEIVED, "bb").unwrap();
    assert_eq!(hm::messages_page(&c, "g:01", None, 10).unwrap()[0].sender, "bb");
    // A lista de conversas continua a contar as por ler (a coluna nova mudou os indices).
    let conv = hm::conversations(&c).unwrap();
    assert_eq!(conv.len(), 2);
    assert!(conv.iter().all(|s| s.unread == 1));
}
