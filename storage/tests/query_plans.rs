//! As consultas que correm a cada mensagem tem de usar indices: sem eles, o
//! tempo de cada uma crescia com o tamanho do historico (medido: confirmar um
//! envio passava de ~5 ms para ~17 ms com 12 000 mensagens).

use qt_storage::*;

fn plan(c: &rusqlite::Connection, sql: &str) -> String {
    let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(3)).unwrap();
    rows.map(|r| r.unwrap()).collect::<Vec<_>>().join(" | ")
}

#[test]
fn consultas_frequentes_usam_indices() {
    let p = std::env::temp_dir().join(format!("qt_plans_{}.sqlite", std::process::id()));
    let _ = std::fs::remove_file(&p);
    let c = open_database_with_key(p.to_str().unwrap(), &[9u8; 32]).unwrap();

    // Confirmar um envio (outbox_sent -> mark_sent).
    let p1 = plan(&c, "UPDATE messages SET status = 1 WHERE msg_id = x'00' AND outgoing = 1 AND status = 0");
    assert!(p1.contains("messages_by_msg_id"), "{p1}");

    // Marcar uma conversa como lida.
    let p2 = plan(&c, "SELECT msg_id FROM messages WHERE contact_id = 'a' AND outgoing = 0 AND status = 10 ORDER BY seq");
    assert!(p2.contains("messages_unread"), "{p2}");

    // Lista de conversas: nenhuma leitura da tabela inteira nem do indice inteiro
    // (desde a 0.9.12 os contactos sao percorridos aos saltos no indice).
    let p3 = plan(
        &c,
        "WITH RECURSIVE k(cid) AS (
             SELECT MIN(contact_id) FROM messages
             UNION ALL
             SELECT (SELECT MIN(contact_id) FROM messages WHERE contact_id > k.cid) FROM k WHERE k.cid IS NOT NULL
         )
         SELECT seq, (SELECT COUNT(*) FROM messages u WHERE u.contact_id = m.contact_id AND u.outgoing = 0 AND u.status = 10)
         FROM k JOIN messages m ON m.seq = (SELECT MAX(seq) FROM messages WHERE contact_id = k.cid)
         WHERE k.cid IS NOT NULL ORDER BY m.seq DESC",
    );
    assert!(!p3.contains("SCAN m ") && !p3.ends_with("SCAN m"), "{p3}");
    assert!(!p3.contains("SCAN messages"), "{p3}");
    assert!(p3.contains("messages_unread"), "{p3}");

    drop(c);
    let _ = std::fs::remove_file(&p);
}
