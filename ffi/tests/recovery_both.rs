//! As DUAS pessoas recuperam a conta (12 palavras) em telemoveis novos e continuam a falar.
//!
//! Caso real (14/11/2026): Nuno e um tester recuperaram as duas contas quase ao mesmo tempo,
//! trocaram mensagens, o relay entregou-as (confirmadas pelas apps) e ninguem as viu.
//!
//! O relay simulado guarda o ultimo bundle publicado por cada conta: enquanto o segundo ainda
//! nao recuperou, o primeiro so encontra o bundle ANTIGO (do telemovel que ja nao existe).

use qt_ffi::recovery::BackupContact;
use qt_ffi::*;
use std::collections::HashMap;
use std::sync::Arc;

fn tmp(n: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_recboth_{}_{}_{}.sqlite", n, std::process::id(), id);
    let _ = std::fs::remove_file(&p);
    p
}

struct Phone {
    c: Arc<QtClient>,
    pid: String,
    cid: String,
}

fn hexs(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn new_phone(name: &str, key: u8) -> Phone {
    let c = QtClient::open(tmp(name), vec![key; 32]).unwrap();
    let id = c.create_identity().unwrap();
    Phone { pid: id.public_id.clone(), cid: hexs(&id.identity_public), c }
}

/// Relay simulado: bundles publicados e filas por conta (cid -> envelopes).
#[derive(Default)]
struct Relay {
    bundles: HashMap<String, Vec<u8>>,
    queues: HashMap<String, Vec<Vec<u8>>>,
}

impl Relay {
    fn publish(&mut self, p: &Phone) {
        self.bundles.insert(p.cid.clone(), p.c.publishable_bundle().unwrap());
    }
    /// A outbox de `from` vai toda para a fila de `to_cid` (o relay nao sabe o que e).
    fn upload(&mut self, from: &Phone, to_cid: &str) {
        for item in from.c.outbox().unwrap() {
            if item.contact_id == to_cid {
                self.queues.entry(to_cid.to_string()).or_default().push(item.blob.clone());
            }
            from.c.outbox_sent(item.outbox_id).unwrap();
        }
    }
    /// O telemovel liga-se e recebe a fila (erros definitivos: a app confirma e descarta).
    fn download(&mut self, to: &Phone) -> usize {
        let mut ok = 0;
        for blob in self.queues.remove(&to.cid).unwrap_or_default() {
            if to.c.receive_sealed(blob).is_ok() {
                ok += 1;
            }
        }
        ok
    }
    /// O que a app faz ao arrancar: abre sessao com quem esta em `sessions_to_restart`.
    fn restart_sessions(&self, p: &Phone, contacts: &[(&str, &str)]) {
        for cid in p.c.sessions_to_restart().unwrap() {
            let Some(bytes) = self.bundles.get(&cid) else { continue };
            let pid = contacts.iter().find(|(c, _)| *c == cid).map(|(_, p)| p.to_string()).unwrap();
            let bundle = peer_bundle_from_relay_bytes(bytes.clone()).unwrap();
            p.c.start_session(pid, bundle).unwrap();
        }
    }
}

fn settle(r: &mut Relay, a: &Phone, b: &Phone) {
    for _ in 0..5 {
        a.c.prepare_delivery_receipts().unwrap();
        b.c.prepare_delivery_receipts().unwrap();
        r.upload(a, &b.cid);
        r.upload(b, &a.cid);
        r.download(b);
        r.download(a);
    }
}

fn texts(p: &Phone, contact: &str) -> Vec<String> {
    p.c.messages_page(contact.to_string(), None, 500).unwrap().into_iter().map(|m| m.text).collect()
}

/// Copia de recuperacao de `p` (as partes que o relay guarda) e as palavras.
fn backup(p: &Phone, other: &Phone) -> (Vec<String>, Vec<Vec<u8>>) {
    use base64::Engine;
    let words = p.c.recovery_enable().unwrap();
    let up = p
        .c
        .recovery_backup(vec![BackupContact { nickname: "o outro".into(), public_id: other.pid.clone() }], true)
        .unwrap();
    let parts = up
        .requests
        .iter()
        .map(|r| {
            let v: serde_json::Value = serde_json::from_slice(r).unwrap();
            base64::engine::general_purpose::STANDARD.decode(v["BackupPut"]["data"].as_str().unwrap()).unwrap()
        })
        .collect();
    p.c.recovery_backup_sent(up.digest).unwrap();
    (words, parts)
}

fn restore(name: &str, key: u8, words: Vec<String>, parts: Vec<Vec<u8>>) -> Phone {
    let c = QtClient::open(tmp(name), vec![key; 32]).unwrap();
    c.relay_auth_message(vec![7u8; 32]).unwrap();
    c.recovery_prepare(words).unwrap();
    let r = c.recovery_restore(parts).unwrap();
    // Como a app ao arrancar: a lista de contactos vai para o nucleo.
    c.call_contacts_sync(r.contacts.iter().map(|x| x.public_id[4..68].to_ascii_lowercase()).collect()).unwrap();
    let id = c.export_public_identity().unwrap();
    assert_eq!(id.public_id, r.public_id);
    Phone { pid: id.public_id.clone(), cid: hexs(&id.identity_public), c }
}

/// Os dois ja falavam; fazem copia.
fn before(tag: &str, ka: u8, kb: u8) -> (Relay, Phone, Phone) {
    let mut r = Relay::default();
    let a = new_phone(&format!("{tag}a"), ka);
    let b = new_phone(&format!("{tag}b"), kb);
    r.publish(&a);
    r.publish(&b);
    let bundle = peer_bundle_from_relay_bytes(r.bundles[&b.cid].clone()).unwrap();
    a.c.start_session(b.pid.clone(), bundle).unwrap();
    a.c.send_text(b.cid.clone(), "antes A".into()).unwrap();
    settle(&mut r, &a, &b);
    b.c.send_text(a.cid.clone(), "antes B".into()).unwrap();
    settle(&mut r, &a, &b);
    assert!(texts(&a, &b.cid).contains(&"antes B".to_string()));
    (r, a, b)
}

fn assert_talk(r: &mut Relay, a: &Phone, b: &Phone, tag: &str) {
    a.c.send_text(b.cid.clone(), format!("{tag} de A")).unwrap();
    b.c.send_text(a.cid.clone(), format!("{tag} de B")).unwrap();
    settle(r, a, b);
    assert!(texts(b, &a.cid).contains(&format!("{tag} de A")), "B nao recebeu '{tag} de A': {:?}", texts(b, &a.cid));
    assert!(texts(a, &b.cid).contains(&format!("{tag} de B")), "A nao recebeu '{tag} de B': {:?}", texts(a, &b.cid));
}

#[test]
fn os_dois_recuperam_um_depois_do_outro() {
    let (mut r, a, b) = before("seq", 0x71, 0x72);
    let (wa, pa) = backup(&a, &b);
    let (wb, pb) = backup(&b, &a);
    let contacts = [(a.cid.as_str(), a.pid.as_str()), (b.cid.as_str(), b.pid.as_str())];

    // A recupera primeiro: publica o bundle novo e abre sessao com o bundle ANTIGO de B.
    let a2 = restore("seqa2", 0x73, wa, pa);
    r.publish(&a2);
    r.restart_sessions(&a2, &contacts);
    a2.c.send_text(b.cid.clone(), "voltei (A)".into()).unwrap();
    r.upload(&a2, &b.cid);

    // Depois B recupera, publica e abre sessao com o bundle novo de A.
    let b2 = restore("seqb2", 0x74, wb, pb);
    r.publish(&b2);
    r.restart_sessions(&b2, &contacts);
    b2.c.send_text(a.cid.clone(), "voltei (B)".into()).unwrap();
    settle(&mut r, &a2, &b2);

    assert!(texts(&a2, &b.cid).contains(&"voltei (B)".to_string()), "A2: {:?}", texts(&a2, &b.cid));
    assert!(texts(&b2, &a.cid).contains(&"voltei (A)".to_string()), "B2: {:?}", texts(&b2, &a.cid));
    assert_talk(&mut r, &a2, &b2, "depois");
}

#[test]
fn os_dois_recuperam_ao_mesmo_tempo() {
    let (mut r, a, b) = before("sim", 0x81, 0x82);
    let (wa, pa) = backup(&a, &b);
    let (wb, pb) = backup(&b, &a);
    let contacts = [(a.cid.as_str(), a.pid.as_str()), (b.cid.as_str(), b.pid.as_str())];

    let a2 = restore("sima2", 0x83, wa, pa);
    let b2 = restore("simb2", 0x84, wb, pb);
    r.publish(&a2);
    r.publish(&b2);
    // Os dois abrem sessao antes de receberem o que quer que seja (sessoes cruzadas).
    r.restart_sessions(&a2, &contacts);
    r.restart_sessions(&b2, &contacts);
    assert_talk(&mut r, &a2, &b2, "ja");
    assert_talk(&mut r, &a2, &b2, "outra vez");
}

#[test]
fn os_dois_recuperam_e_o_primeiro_escreve_antes_de_o_outro_publicar() {
    // As identidades sao aleatorias: em varias voltas o desempate das sessoes cai para os dois
    // lados (antes da correcao, metade das voltas ficava sem mensagens para sempre).
    for i in 0..8u8 {
        late_round(i);
    }
}

fn late_round(i: u8) {
    // Pior caso: A recupera e escreve VARIAS vezes com o bundle antigo de B; B so recupera
    // depois e nao abre sessao (sessions_to_restart sem bundle de A, p.ex. A ainda sem rede).
    let (mut r, a, b) = before(&format!("late{i}"), 0x91, 0x92);
    let (wa, pa) = backup(&a, &b);
    let (wb, pb) = backup(&b, &a);
    let contacts = [(a.cid.as_str(), a.pid.as_str()), (b.cid.as_str(), b.pid.as_str())];

    let a2 = restore(&format!("latea2{i}"), 0x93, wa, pa);
    r.restart_sessions(&a2, &contacts); // bundle antigo de B
    for k in 0..3 {
        a2.c.send_text(b.cid.clone(), format!("A{k}")).unwrap();
    }
    r.upload(&a2, &b.cid);
    let b2 = restore(&format!("lateb2{i}"), 0x94, wb, pb);
    r.download(&b2); // B2 recebe o que A2 cifrou para o telemovel antigo
    r.publish(&a2);
    r.publish(&b2);
    r.restart_sessions(&b2, &contacts);
    r.restart_sessions(&a2, &contacts);
    settle(&mut r, &a2, &b2);
    assert_talk(&mut r, &a2, &b2, &format!("agora{i}"));
    for k in 0..3 {
        assert!(texts(&b2, &a.cid).contains(&format!("A{k}")), "volta {i}: B2 perdeu A{k}: {:?}", texts(&b2, &a.cid));
    }
}
