//! Badges (tipo 13): certificado do emissor, so contactos, cifrado, sem nada no relay.

use qt_core::badge::{
    issue, issuer_public, BADGE_BUGHUNTER, BADGE_DEV, BADGE_FOUNDER, BADGE_LEGACY, BADGE_NONE,
    BADGE_SUPPORT, BADGE_TESTER, BADGE_TROLL,
};
use qt_ffi::history::*;
use qt_ffi::*;
use std::sync::Arc;

const SEED: [u8; 32] = [0x5e; 32];

fn tmp(n: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_badge_{}_{}_{}.sqlite", n, std::process::id(), id);
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
        self.id
            .identity_public
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
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
    (a, b)
}

fn setup() {
    qt_ffi::badges::set_test_issuer(issuer_public(&SEED));
}

fn subject(p: &Peer) -> [u8; 32] {
    p.id.identity_public.clone().try_into().unwrap()
}

/// O emissor (outra identidade, como a ferramenta `badge`) envia um certificado a `to`.
fn grant(issuer: &Peer, to: &Peer, badge: u8, at: i64) -> Vec<ReceiveResult> {
    if !issuer.c.has_session(to.cid()).unwrap() {
        let bundle = peer_bundle_from_relay_bytes(to.c.publishable_bundle().unwrap()).unwrap();
        issuer
            .c
            .start_session(to.id.public_id.clone(), bundle)
            .unwrap();
    }
    issuer
        .c
        .badge_deliver(to.cid(), issue(&SEED, &subject(to), badge, at))
        .unwrap();
    relay(issuer, to)
}

#[test]
fn badge_do_emissor_chega_e_segue_para_os_contactos() {
    setup();
    let (a, b) = pair();
    let nuno = peer("emissor", 0x73);
    assert_eq!(a.c.badge_mine().unwrap(), BADGE_NONE);
    // Sem badge, nada a enviar.
    assert_eq!(a.c.badge_sync(vec![b.cid()]).unwrap(), 0);

    let r = grant(&nuno, &a, BADGE_TESTER, 1_000);
    assert!(matches!(r[..], [ReceiveResult::Updated { .. }]));
    assert_eq!(a.c.badge_mine().unwrap(), BADGE_TESTER);
    // O emissor nao aparece como conversa.
    assert!(a
        .c
        .conversations()
        .unwrap()
        .iter()
        .all(|c| c.contact_id != nuno.cid()));

    assert_eq!(a.c.badge_sync(vec![b.cid()]).unwrap(), 1);
    assert_eq!(
        a.c.badge_sync(vec![b.cid()]).unwrap(),
        0,
        "so uma vez por versao"
    );
    relay(&a, &b);
    assert_eq!(b.c.badge_of(a.cid()).unwrap(), BADGE_TESTER);

    // Um certificado mais antigo nao substitui o atual.
    grant(&nuno, &a, BADGE_FOUNDER, 500);
    assert_eq!(a.c.badge_mine().unwrap(), BADGE_TESTER);

    // Mudar e retirar: o mais recente vale e segue para os contactos.
    grant(&nuno, &a, BADGE_DEV, 2_000);
    a.c.badge_sync(vec![b.cid()]).unwrap();
    relay(&a, &b);
    assert_eq!(b.c.badge_of(a.cid()).unwrap(), BADGE_DEV);
    grant(&nuno, &a, BADGE_NONE, 3_000);
    assert_eq!(a.c.badge_mine().unwrap(), BADGE_NONE);
    assert_eq!(a.c.badge_sync(vec![b.cid()]).unwrap(), 1);
    relay(&a, &b);
    assert_eq!(b.c.badge_of(a.cid()).unwrap(), BADGE_NONE);
}

#[test]
fn nao_se_copia_nem_se_inventa_um_badge() {
    setup();
    let (a, b) = pair();
    let nuno = peer("emissor2", 0x74);
    // O certificado da Alice nao pode ser entregue a outra identidade.
    let cert_a = issue(&SEED, &subject(&a), BADGE_FOUNDER, 1_000);
    let bundle = peer_bundle_from_relay_bytes(b.c.publishable_bundle().unwrap()).unwrap();
    nuno.c
        .start_session(b.id.public_id.clone(), bundle)
        .unwrap();
    assert!(nuno.c.badge_deliver(b.cid(), cert_a.clone()).is_err());
    // Assinado por outra chave: recusado.
    let falso = issue(&[1u8; 32], &subject(&b), BADGE_FOUNDER, 1_000);
    assert!(nuno.c.badge_deliver(b.cid(), falso).is_err());
    // O Bob recebe o certificado da Alice e tenta passa-lo como seu aos contactos: a Alice
    // ve que nao e da identidade dele e ignora.
    grant(&nuno, &a, BADGE_FOUNDER, 1_000);
    a.c.badge_sync(vec![b.cid()]).unwrap();
    relay(&a, &b);
    assert_eq!(b.c.badge_mine().unwrap(), BADGE_NONE);
    assert_eq!(b.c.badge_of(a.cid()).unwrap(), BADGE_FOUNDER);
    assert_eq!(a.c.badge_of(b.cid()).unwrap(), BADGE_NONE);
}

#[test]
fn apagar_o_contacto_esquece_o_badge() {
    setup();
    let (a, b) = pair();
    let nuno = peer("emissor3", 0x75);
    grant(&nuno, &a, BADGE_DEV, 1_000);
    a.c.badge_sync(vec![b.cid()]).unwrap();
    relay(&a, &b);
    assert_eq!(b.c.badge_of(a.cid()).unwrap(), BADGE_DEV);
    b.c.delete_contact(a.cid()).unwrap();
    assert_eq!(b.c.badge_of(a.cid()).unwrap(), BADGE_NONE);
}

#[test]
fn badges_novos_chegam_aos_contactos() {
    setup();
    let (a, b) = pair();
    let nuno = peer("emissor4", 0x76);
    let mut at = 1_000;
    for badge in [BADGE_BUGHUNTER, BADGE_SUPPORT, BADGE_TROLL, BADGE_LEGACY] {
        at += 1_000;
        grant(&nuno, &a, badge, at);
        assert_eq!(a.c.badge_mine().unwrap(), badge);
        assert_eq!(a.c.badge_sync(vec![b.cid()]).unwrap(), 1);
        relay(&a, &b);
        assert_eq!(b.c.badge_of(a.cid()).unwrap(), badge);
    }
    // Um numero que ninguem conhece (10) e recusado: fica a Legacy.
    let falso = issue(&SEED, &subject(&a), BADGE_LEGACY + 1, at + 1_000);
    assert!(nuno.c.badge_deliver(a.cid(), falso).is_err());
    assert_eq!(a.c.badge_mine().unwrap(), BADGE_LEGACY);
}
