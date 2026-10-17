//! Presenca (online / offline): so contactos, cifrada, so em memoria.

use qt_ffi::history::*;
use qt_ffi::*;
use std::sync::Arc;

fn tmp(n: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_pres_{}_{}_{}.sqlite", n, std::process::id(), id);
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

/// O relay: entrega a outbox de `from` a `to` e confirma. Devolve os resultados.
fn relay(from: &Peer, to: &Peer) -> Vec<ReceiveResult> {
    let mut out = Vec::new();
    for item in from.c.outbox().unwrap() {
        if item.contact_id == to.cid() {
            out.push(to.c.receive_sealed(item.blob.clone()).unwrap());
        }
        from.c.outbox_sent(item.outbox_id).unwrap();
    }
    out
}

/// Alice e Bob ja falaram (ha sessao nos dois sentidos) e sao contactos um do outro.
fn pair() -> (Peer, Peer) {
    let (a, b) = (peer("a", 0x71), peer("b", 0x72));
    let bundle = peer_bundle_from_relay_bytes(b.c.publishable_bundle().unwrap()).unwrap();
    a.c.start_session(b.id.public_id.clone(), bundle).unwrap();
    a.c.send_text(b.cid(), "olá".into()).unwrap();
    relay(&a, &b);
    b.c.send_text(a.cid(), "olá!".into()).unwrap();
    relay(&b, &a);
    a.c.presence_configure(true, vec![b.cid()]).unwrap();
    b.c.presence_configure(true, vec![a.cid()]).unwrap();
    (a, b)
}

#[test]
fn abrir_a_app_mostra_online_e_o_outro_responde() {
    let (a, b) = pair();
    b.c.presence_announce(true).unwrap();
    relay(&b, &a); // A ainda esta com a app fechada: ve o Bob, mas nao responde
    assert_eq!(a.c.presence_online(), vec![b.cid()]);
    assert!(a.c.outbox().unwrap().is_empty());

    // A Alice abre a app: diz ao Bob, que (com a app aberta) ja lhe tinha dito.
    assert_eq!(a.c.presence_announce(true).unwrap(), 1);
    let r = relay(&a, &b);
    assert!(matches!(&r[0], ReceiveResult::Updated { contact_id } if *contact_id == a.cid()));
    assert_eq!(b.c.presence_online(), vec![a.cid()]);
    // O Bob mandou o "online" ha menos de 30 s: nao responde outra vez (sem pingue-pongue).
    assert!(b.c.outbox().unwrap().is_empty());

    // A Alice fecha a app: o Bob deixa de a ver online.
    assert_eq!(a.c.presence_announce(false).unwrap(), 1);
    relay(&a, &b);
    assert!(b.c.presence_online().is_empty());
}

#[test]
fn o_batimento_mantem_online_quem_ja_se_conhece() {
    let (a, b) = pair();
    // O "online" do Bob perdeu-se (nunca chegou a Alice).
    b.c.presence_announce(true).unwrap();
    for item in b.c.outbox().unwrap() {
        b.c.outbox_sent(item.outbox_id).unwrap();
    }
    a.c.presence_announce(true).unwrap();
    relay(&a, &b);
    // O Bob mandou o dele ha menos de 30 s, por isso nao responde; o batimento seguinte chega.
    assert!(b.c.outbox().unwrap().is_empty());
    assert_eq!(b.c.presence_heartbeat().unwrap(), 1);
    relay(&b, &a);
    assert_eq!(a.c.presence_online(), vec![b.cid()]);
}

#[test]
fn so_contactos() {
    let (a, b) = pair();
    // O Bob tira a Alice dos contactos: o que ela manda e ignorado, e ele nao responde.
    b.c.presence_configure(true, vec![]).unwrap();
    b.c.presence_announce(true).unwrap();
    assert!(b.c.outbox().unwrap().is_empty(), "a ninguem");
    a.c.presence_announce(true).unwrap();
    let r = relay(&a, &b);
    assert!(matches!(&r[0], ReceiveResult::NoChange { .. }));
    assert!(b.c.presence_online().is_empty());
    assert!(b.c.outbox().unwrap().is_empty());
    // E nada na conversa (nem aviso de "atualiza a app").
    assert_eq!(b.c.messages_page(a.cid(), None, 10).unwrap().len(), 2);
}

#[test]
fn desligar_avisa_offline_e_deixa_de_ver() {
    let (a, b) = pair();
    a.c.presence_announce(true).unwrap();
    relay(&a, &b);
    b.c.presence_announce(true).unwrap();
    relay(&b, &a);
    assert_eq!(b.c.presence_online(), vec![a.cid()]);
    assert_eq!(a.c.presence_online(), vec![b.cid()]);

    // A Alice desliga: o Bob recebe "offline"; a Alice deixa de ver o Bob.
    assert_eq!(a.c.presence_configure(false, vec![b.cid()]).unwrap(), 1);
    relay(&a, &b);
    assert!(b.c.presence_online().is_empty());
    assert!(a.c.presence_online().is_empty());
    assert_eq!(a.c.presence_announce(true).unwrap(), 0, "desligada: nao anuncia");
    b.c.presence_heartbeat().unwrap();
    relay(&b, &a);
    assert!(a.c.presence_online().is_empty(), "desligada: nao mostra");
}

#[test]
fn abrir_e_fechar_seguido_nao_repete() {
    let (a, b) = pair();
    assert_eq!(a.c.presence_announce(true).unwrap(), 1);
    assert_eq!(a.c.presence_announce(true).unwrap(), 0, "menos de 1 min depois");
    assert_eq!(a.c.presence_heartbeat().unwrap(), 0, "ainda nao sei se o Bob esta online");
    let _ = b;
}

#[test]
fn bloqueado_nao_recebe_nem_e_mostrado() {
    let (a, b) = pair();
    b.c.block_contact(a.cid()).unwrap();
    a.c.presence_announce(true).unwrap();
    relay(&a, &b);
    assert!(b.c.presence_online().is_empty());
    b.c.presence_announce(true).unwrap();
    assert!(b.c.outbox().unwrap().is_empty());
}
