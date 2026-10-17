//! Historico persistente + estados (Enviado / Entregue / Visto) + outbox,
//! com dois clientes reais (Alice e Bob). O "relay" e simulado: os envelopes
//! passam de um lado para o outro tal como o relay os entregaria.

use qt_ffi::history::*;
use qt_ffi::*;
use std::sync::Arc;

fn tmp(n: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_hist_{}_{}_{}.sqlite", n, std::process::id(), id);
    let _ = std::fs::remove_file(&p);
    p
}

struct Peer {
    path: String,
    key: [u8; 32],
    c: Arc<QtClient>,
    id: PublicIdentity,
}

fn peer(name: &str, key: [u8; 32]) -> Peer {
    let path = tmp(name);
    let c = QtClient::open(path.clone(), key.to_vec()).unwrap();
    let id = c.create_identity().unwrap();
    Peer { path, key, c, id }
}

impl Peer {
    fn restart(&mut self) {
        self.c = QtClient::open(self.path.clone(), self.key.to_vec()).unwrap();
    }
    fn cid(&self) -> String {
        self.id.identity_public.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Alice inicia a sessao com o bundle do Bob (como faria com o bundle do relay).
fn setup() -> (Peer, Peer) {
    let a = peer("alice", [0x11; 32]);
    let b = peer("bob", [0x22; 32]);
    let bundle = peer_bundle_from_relay_bytes(b.c.publishable_bundle().unwrap()).unwrap();
    a.c.start_session(b.id.public_id.clone(), bundle).unwrap();
    (a, b)
}

/// Simula o relay: entrega toda a outbox de `from` a `to` e confirma o envio.
fn deliver_outbox(from: &Peer, to: &Peer) -> Vec<ReceiveResult> {
    let mut out = Vec::new();
    for item in from.c.outbox().unwrap() {
        out.push(to.c.receive_sealed(item.blob.clone()).unwrap());
        from.c.outbox_sent(item.outbox_id).unwrap();
    }
    out
}

fn status_of(p: &Peer, contact: &str, id: &str) -> MessageStatus {
    p.c.message(contact.to_string(), id.to_string()).unwrap().unwrap().status
}

#[test]
fn ciclo_completo_enviado_entregue_visto() {
    let (a, b) = setup();
    let (alice, bob) = (a.cid(), b.cid());

    let item = a.c.send_text(bob.clone(), "olá Bob".into()).unwrap();
    assert_eq!(status_of(&a, &bob, &item.outbox_id), MessageStatus::Pending);
    // O convite (o Public ID da Alice) vai antes da 1.a mensagem.
    assert_eq!(a.c.outbox().unwrap().len(), 2);

    // Relay recebe e confirma -> "Enviado"; Bob recebe e guarda.
    let r = deliver_outbox(&a, &b);
    assert_eq!(status_of(&a, &bob, &item.outbox_id), MessageStatus::Sent);
    assert!(matches!(&r[0], ReceiveResult::NoChange { .. }), "o convite nao aparece no historico");
    match &r[1] {
        ReceiveResult::NewMessage { message } => {
            assert_eq!(message.text, "olá Bob");
            assert_eq!(message.contact_id, alice);
            assert_eq!(message.status, MessageStatus::Received);
            assert_eq!(message.message_id, item.outbox_id, "o id e o mesmo dos dois lados");
        }
        other => panic!("{other:?}"),
    }
    assert!(a.c.outbox().unwrap().is_empty());

    // Bob envia o recibo "entregue" -> Alice ve "Entregue".
    assert_eq!(b.c.prepare_delivery_receipts().unwrap(), 1);
    assert_eq!(b.c.prepare_delivery_receipts().unwrap(), 0, "nao duplica recibos");
    let r = deliver_outbox(&b, &a);
    assert!(matches!(&r[0], ReceiveResult::StatusChanged { status: MessageStatus::Delivered, .. }));
    assert_eq!(status_of(&a, &bob, &item.outbox_id), MessageStatus::Delivered);

    // Bob abre a conversa -> recibo "visto" -> Alice ve "Visto".
    let conv = b.c.conversations().unwrap();
    assert_eq!(conv[0].unread, 1);
    assert_eq!(b.c.mark_conversation_read(alice.clone(), true).unwrap(), 1);
    assert_eq!(b.c.conversations().unwrap()[0].unread, 0);
    deliver_outbox(&b, &a);
    assert_eq!(status_of(&a, &bob, &item.outbox_id), MessageStatus::Read);

    // Os recibos nao aparecem no historico de ninguem.
    assert_eq!(a.c.messages_page(bob.clone(), None, 50).unwrap().len(), 1);
    assert_eq!(b.c.messages_page(alice.clone(), None, 50).unwrap().len(), 1);
}

#[test]
fn recibo_de_leitura_desligado_nao_envia_nada() {
    let (a, b) = setup();
    a.c.send_text(b.cid(), "x".into()).unwrap();
    deliver_outbox(&a, &b);
    assert_eq!(b.c.mark_conversation_read(a.cid(), false).unwrap(), 1);
    assert!(b.c.outbox().unwrap().is_empty());
}

#[test]
fn reentrega_pelo_relay_nao_duplica_mensagens() {
    let (a, b) = setup();
    let item = a.c.send_text(b.cid(), "uma vez".into()).unwrap();
    // O relay entrega, a app do Bob falha antes do Ack e o relay volta a entregar.
    assert!(matches!(b.c.receive_sealed(item.blob.clone()).unwrap(), ReceiveResult::NewMessage { .. }));
    assert!(matches!(b.c.receive_sealed(item.blob.clone()), Err(QtError::Replay)));
    // A app de envio tambem pode reenviar (outbox) antes de receber o Ack: o mesmo envelope.
    assert_eq!(b.c.messages_page(a.cid(), None, 10).unwrap().len(), 1);
}

#[test]
fn historico_e_outbox_sobrevivem_ao_reinicio() {
    let (mut a, mut b) = setup();
    let bob = b.cid();
    let alice = a.cid();
    for i in 0..30 {
        a.c.send_text(bob.clone(), format!("msg {i}")).unwrap();
    }
    // A app da Alice "morre" antes de conseguir enviar: tudo continua na outbox.
    a.restart();
    let pend = a.c.outbox().unwrap();
    assert_eq!(pend.len(), 31, "30 mensagens + o convite");
    for it in &pend {
        b.c.receive_sealed(it.blob.clone()).unwrap();
        a.c.outbox_sent(it.outbox_id.clone()).unwrap();
    }
    b.restart();
    let p1 = b.c.messages_page(alice.clone(), None, 20).unwrap();
    assert_eq!(p1.len(), 20);
    assert_eq!(p1[0].text, "msg 29");
    let p2 = b.c.messages_page(alice.clone(), Some(p1.last().unwrap().seq), 20).unwrap();
    assert_eq!(p2.len(), 10);
    assert_eq!(p2.last().unwrap().text, "msg 0");
    // Apos o reinicio, a ordem e os estados mantem-se.
    a.restart();
    let mine = a.c.messages_page(bob.clone(), None, 50).unwrap();
    assert_eq!(mine.len(), 30);
    assert!(mine.iter().all(|m| m.outgoing && m.status == MessageStatus::Sent));
}

#[test]
fn conversa_nos_dois_sentidos_e_lista_de_conversas() {
    let (a, b) = setup();
    let (alice, bob) = (a.cid(), b.cid());
    a.c.send_text(bob.clone(), "1".into()).unwrap();
    deliver_outbox(&a, &b);
    b.c.send_text(alice.clone(), "2".into()).unwrap();
    deliver_outbox(&b, &a);
    a.c.send_text(bob.clone(), "3".into()).unwrap();
    deliver_outbox(&a, &b);
    let hist: Vec<String> = b.c.messages_page(alice.clone(), None, 10).unwrap().into_iter().rev().map(|m| m.text).collect();
    assert_eq!(hist, vec!["1", "2", "3"]);
    let conv = a.c.conversations().unwrap();
    assert_eq!(conv.len(), 1);
    assert_eq!(conv[0].last_message.text, "3");
    assert!(conv[0].last_message.outgoing);
    // messages_after: so o que chegou depois.
    let first = b.c.messages_page(alice.clone(), None, 10).unwrap().last().unwrap().seq;
    assert_eq!(b.c.messages_after(alice, first).unwrap().len(), 2);
}

#[test]
fn texto_vazio_ou_grande_demais_e_recusado() {
    let (a, b) = setup();
    assert!(a.c.send_text(b.cid(), "   ".into()).is_err());
    assert!(a.c.send_text(b.cid(), "x".repeat(9_001)).is_err());
    assert!(a.c.send_text(b.cid(), "x".repeat(9_000)).is_ok());
}

#[test]
fn cliente_antigo_com_texto_simples_continua_a_funcionar() {
    let (a, b) = setup();
    // A API antiga (encrypt_sealed com texto simples) ainda e aceite pela nova rececao.
    let blob = a.c.encrypt_sealed(b.cid(), b"texto antigo".to_vec()).unwrap();
    match b.c.receive_sealed(blob).unwrap() {
        ReceiveResult::NewMessage { message } => assert_eq!(message.text, "texto antigo"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn falha_a_meio_nao_perde_a_mensagem() {
    // Se a rececao falhar (ex.: envelope corrompido), nada fica gravado e o
    // envelope original continua utilizavel.
    let (a, b) = setup();
    let item = a.c.send_text(b.cid(), "importante".into()).unwrap();
    let mut bad = item.blob.clone();
    let n = bad.len();
    bad[n - 1] ^= 0xff;
    assert!(b.c.receive_sealed(bad).is_err());
    assert!(b.c.messages_page(a.cid(), None, 10).unwrap().is_empty());
    assert!(matches!(b.c.receive_sealed(item.blob).unwrap(), ReceiveResult::NewMessage { .. }));
}

#[test]
fn convite_leva_o_id_de_quem_inicia_e_so_vai_uma_vez() {
    let (a, b) = setup();
    let (alice, bob) = (a.cid(), b.cid());
    a.c.send_text(bob.clone(), "olá".into()).unwrap();
    deliver_outbox(&a, &b);
    assert_eq!(b.c.pending_intro(alice.clone()).unwrap(), Some(a.id.public_id.clone()));
    assert_eq!(b.c.pending_intros().unwrap(), vec![alice.clone()]);

    // A seguinte ja nao leva convite.
    a.c.send_text(bob.clone(), "outra".into()).unwrap();
    assert_eq!(a.c.outbox().unwrap().len(), 1);
    deliver_outbox(&a, &b);

    // Quem responde nao manda convite (nao iniciou a sessao).
    b.c.send_text(alice.clone(), "resposta".into()).unwrap();
    assert_eq!(b.c.outbox().unwrap().len(), 1);
    deliver_outbox(&b, &a);
    assert!(a.c.pending_intros().unwrap().is_empty());

    // Aceite (a app guardou o contacto): o convite sai.
    b.c.dismiss_intro(alice.clone()).unwrap();
    assert_eq!(b.c.pending_intro(alice.clone()).unwrap(), None);
}

#[test]
fn recusar_apagando_o_contacto_limpa_o_convite() {
    let (a, b) = setup();
    let alice = a.cid();
    a.c.send_text(b.cid(), "olá".into()).unwrap();
    deliver_outbox(&a, &b);
    b.c.delete_contact(alice.clone()).unwrap();
    assert_eq!(b.c.pending_intro(alice).unwrap(), None);
}

#[test]
fn reacoes_nos_dois_sentidos_trocar_e_retirar() {
    let (a, b) = setup();
    let (alice, bob) = (a.cid(), b.cid());
    let item = a.c.send_text(bob.clone(), "olá".into()).unwrap();
    deliver_outbox(&a, &b);
    let mid = item.outbox_id.clone();

    // O Bob reage à mensagem da Alice.
    b.c.react(alice.clone(), mid.clone(), "❤️".into()).unwrap();
    let mine = b.c.message(alice.clone(), mid.clone()).unwrap().unwrap().reactions;
    assert_eq!(mine, vec![Reaction { emoji: "❤️".into(), mine: true, from_contact: String::new() }]);
    let r = deliver_outbox(&b, &a);
    assert!(matches!(&r[0], ReceiveResult::Updated { contact_id } if *contact_id == bob));
    let got = a.c.messages_page(bob.clone(), None, 10).unwrap();
    assert_eq!(got[0].reactions, vec![Reaction { emoji: "❤️".into(), mine: false, from_contact: bob.clone() }]);

    // A Alice também reage (à sua própria mensagem): ficam as duas.
    a.c.react(bob.clone(), mid.clone(), "👍".into()).unwrap();
    deliver_outbox(&a, &b);
    assert_eq!(b.c.message(alice.clone(), mid.clone()).unwrap().unwrap().reactions.len(), 2);

    // O Bob troca e depois retira a dele.
    b.c.react(alice.clone(), mid.clone(), "😂".into()).unwrap();
    deliver_outbox(&b, &a);
    assert!(a.c.message(bob.clone(), mid.clone()).unwrap().unwrap().reactions.iter().any(|r| r.emoji == "😂" && !r.mine));
    b.c.react(alice.clone(), mid.clone(), String::new()).unwrap();
    deliver_outbox(&b, &a);
    let left = a.c.message(bob.clone(), mid.clone()).unwrap().unwrap().reactions;
    assert_eq!(left, vec![Reaction { emoji: "👍".into(), mine: true, from_contact: String::new() }]);

    // Reação a uma mensagem que não existe: recusada.
    assert!(b.c.react(alice.clone(), "00".repeat(16), "👍".into()).is_err());
    // Emoji grande demais: recusado.
    assert!(b.c.react(alice.clone(), mid.clone(), "😀".repeat(20)).is_err());

    // Apagar a conversa apaga as reações.
    b.c.delete_conversation(alice.clone()).unwrap();
    assert!(b.c.messages_page(alice, None, 10).unwrap().is_empty());
}
