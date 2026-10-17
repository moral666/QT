//! Bloquear contactos: as mensagens de quem esta bloqueado sao descartadas ao
//! chegar (sem historico nem recibos), nao se consegue enviar-lhe nada, e ao
//! desbloquear a conversa continua a funcionar. O "relay" e simulado.

use qt_ffi::history::*;
use qt_ffi::*;
use std::sync::Arc;

fn tmp(n: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_block_{}_{}_{}.sqlite", n, std::process::id(), id);
    let _ = std::fs::remove_file(&p);
    p
}

struct Peer {
    c: Arc<QtClient>,
    id: PublicIdentity,
}

fn peer(name: &str, key: u8) -> Peer {
    let c = QtClient::open(tmp(name), vec![key; 32]).unwrap();
    let id = c.create_identity().unwrap();
    Peer { c, id }
}

impl Peer {
    fn cid(&self) -> String {
        self.id.identity_public.iter().map(|b| format!("{b:02x}")).collect()
    }
}

fn deliver_outbox(from: &Peer, to: &Peer) -> Vec<ReceiveResult> {
    let mut out = Vec::new();
    for item in from.c.outbox().unwrap() {
        out.push(to.c.receive_sealed(item.blob.clone()).unwrap());
        from.c.outbox_sent(item.outbox_id).unwrap();
    }
    out
}

fn setup() -> (Peer, Peer) {
    let a = peer("alice", 0x11);
    let b = peer("bob", 0x22);
    let bundle = peer_bundle_from_relay_bytes(b.c.publishable_bundle().unwrap()).unwrap();
    a.c.start_session(b.id.public_id.clone(), bundle).unwrap();
    // Primeira mensagem: o Bob fica com sessao para responder.
    a.c.send_text(b.cid(), "olá".into()).unwrap();
    deliver_outbox(&a, &b);
    (a, b)
}

#[test]
fn bloqueado_nao_entra_nao_recebe_recibos_e_nao_se_lhe_escreve() {
    let (a, b) = setup();
    let alice = a.cid();

    // Uma resposta do Bob ficou por enviar quando ele bloqueia: sai da outbox.
    b.c.send_text(alice.clone(), "resposta por enviar".into()).unwrap();
    b.c.prepare_delivery_receipts().unwrap();
    assert!(!b.c.outbox().unwrap().is_empty());
    b.c.block_contact(alice.clone()).unwrap();
    assert!(b.c.outbox().unwrap().is_empty(), "o que estava por enviar ao bloqueado sai da outbox");
    assert!(b.c.is_blocked(alice.clone()).unwrap());
    assert_eq!(b.c.blocked_contacts().unwrap(), vec![alice.clone()]);

    let antes = b.c.messages_page(alice.clone(), None, 100).unwrap().len();
    a.c.send_text(b.cid(), "estás aí?".into()).unwrap();
    let r = deliver_outbox(&a, &b);
    assert!(matches!(r[0], ReceiveResult::NoChange { .. }), "a mensagem do bloqueado e descartada");
    assert_eq!(b.c.messages_page(alice.clone(), None, 100).unwrap().len(), antes, "nada entra no historico");

    // Sem recibos para o bloqueado (nem entregue nem visto), e abrir a conversa nao da erro.
    assert_eq!(b.c.prepare_delivery_receipts().unwrap(), 0);
    b.c.mark_conversation_read(alice.clone(), true).unwrap();
    assert!(b.c.outbox().unwrap().is_empty());

    // Nao se consegue enviar ao bloqueado.
    assert!(matches!(b.c.send_text(alice.clone(), "x".into()), Err(QtError::InvalidInput { ref field }) if field == "blocked"));
}

#[test]
fn desbloquear_volta_a_funcionar_nos_dois_sentidos() {
    let (a, b) = setup();
    let alice = a.cid();
    b.c.block_contact(alice.clone()).unwrap();
    a.c.send_text(b.cid(), "perdida".into()).unwrap();
    deliver_outbox(&a, &b);

    b.c.unblock_contact(alice.clone()).unwrap();
    assert!(!b.c.is_blocked(alice.clone()).unwrap());
    assert!(b.c.blocked_contacts().unwrap().is_empty());

    // A sessao continuou a avancar enquanto estava bloqueado: a seguinte entra.
    a.c.send_text(b.cid(), "e agora?".into()).unwrap();
    let r = deliver_outbox(&a, &b);
    match &r[0] {
        ReceiveResult::NewMessage { message } => assert_eq!(message.text, "e agora?"),
        other => panic!("{other:?}"),
    }
    // E o Bob volta a conseguir responder.
    b.c.send_text(alice.clone(), "já cá estou".into()).unwrap();
    let r = deliver_outbox(&b, &a);
    assert!(r.iter().any(|x| matches!(x, ReceiveResult::NewMessage { message } if message.text == "já cá estou")));
}

#[test]
fn bloquear_um_id_invalido_falha() {
    let b = peer("bob", 0x33);
    assert!(b.c.block_contact("nao-e-um-id".into()).is_err());
}
