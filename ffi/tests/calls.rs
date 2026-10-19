//! Chamadas de voz (fase 1): sinalizacao dentro do ratchet e pacotes de audio
//! cifrados, entre dois clientes, sem audio real e sem relay de UDP (o "relay"
//! das mensagens e simulado; os pacotes vao por UDP local, de um socket ao outro).

use qt_ffi::call::*;
use qt_ffi::history::*;
use qt_ffi::*;
use std::net::UdpSocket;
use std::sync::Arc;
use std::time::Duration;

fn tmp(n: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_call_{}_{}_{}.sqlite", n, std::process::id(), id);
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
    fn events(&self) -> Vec<CallEvent> {
        self.c.call_events().unwrap()
    }
}

/// O relay das mensagens: entrega a outbox de `from` a `to` e confirma.
/// Devolve quantos envelopes entregou e se algum tocava a campainha.
fn relay(from: &Peer, to: &Peer) -> (usize, bool) {
    let (mut n, mut ring) = (0, false);
    for item in from.c.outbox().unwrap() {
        if item.contact_id == to.cid() {
            let r = to.c.receive_sealed(item.blob.clone()).unwrap();
            assert!(matches!(r, ReceiveResult::NoChange { .. } | ReceiveResult::Updated { .. } | ReceiveResult::NewMessage { .. }));
            n += 1;
            ring |= !item.silent;
        }
        from.c.outbox_sent(item.outbox_id).unwrap();
    }
    (n, ring)
}

fn connect(a: &Peer, b: &Peer) {
    let bundle = peer_bundle_from_relay_bytes(b.c.publishable_bundle().unwrap()).unwrap();
    a.c.start_session(b.id.public_id.clone(), bundle).unwrap();
    a.c.send_text(b.cid(), "olá".into()).unwrap();
    relay(a, b);
    b.c.send_text(a.cid(), "olá!".into()).unwrap();
    relay(b, a);
    // Descarta recibos que ainda estejam na outbox.
    relay(a, b);
}

fn pair() -> (Peer, Peer) {
    let (a, b) = (peer("a", 0x81), peer("b", 0x82));
    connect(&a, &b);
    (a, b)
}

/// Liga de `a` para `b` e atende. Devolve o call_id.
fn answered_call(a: &Peer, b: &Peer) -> String {
    let start = a.c.call_start(b.cid()).unwrap();
    relay(a, b);
    assert_eq!(b.events(), vec![CallEvent::Incoming { call_id: start.call_id.clone(), contact_id: a.cid() }]);
    b.c.call_accept(start.call_id.clone()).unwrap();
    relay(b, a);
    b.events();
    a.events();
    start.call_id
}

#[test]
fn ligar_tocar_atender_falar_desligar() {
    let (a, b) = pair();
    let history_before = b.c.messages_page(a.cid(), None, 50).unwrap().len();
    let start = a.c.call_start(b.cid()).unwrap();
    assert!(!start.item.silent, "a oferta toca a campainha, como uma mensagem");
    assert_eq!(start.item.wake, Some(CallWake::Call), "campainha marcada como chamada");
    // Tambem quando sai da outbox (enviada mais tarde).
    let queued = a.c.outbox().unwrap();
    assert_eq!(queued.iter().find(|i| i.outbox_id == start.item.outbox_id).and_then(|i| i.wake), Some(CallWake::Call));
    assert_eq!(a.c.call_current().unwrap(), Some(start.call_id.clone()));
    // Antes de atender nao ha chaves de audio.
    assert!(a.c.call_seal(start.call_id.clone(), vec![1]).is_err());

    let (n, ring) = relay(&a, &b);
    assert_eq!((n, ring), (1, true));
    assert_eq!(b.events(), vec![CallEvent::Incoming { call_id: start.call_id.clone(), contact_id: a.cid() }]);
    // Nada aparece no historico do Bob (a chamada nao e uma mensagem).
    assert_eq!(b.c.messages_page(a.cid(), None, 50).unwrap().len(), history_before);

    // O telemovel do Bob avisou que esta a tocar (sem campainha).
    assert_eq!(relay(&b, &a), (1, false));
    assert_eq!(a.events(), vec![CallEvent::Ringing { call_id: start.call_id.clone() }]);

    let answer = b.c.call_accept(start.call_id.clone()).unwrap();
    assert!(answer.silent);
    assert_eq!(answer.wake, None);
    assert_eq!(b.events(), vec![CallEvent::Answered { call_id: start.call_id.clone() }]);
    relay(&b, &a);
    assert_eq!(a.events(), vec![CallEvent::Answered { call_id: start.call_id.clone() }]);

    // Audio (frames falsos) pelos dois sentidos, por UDP local.
    let sa = UdpSocket::bind("127.0.0.1:0").unwrap();
    let sb = UdpSocket::bind("127.0.0.1:0").unwrap();
    sa.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    sb.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut buf = [0u8; 2048];
    let mut sizes = std::collections::HashSet::new();
    for i in 0..50u8 {
        let frame = vec![i; (i as usize) % 100];
        let p = a.c.call_seal(start.call_id.clone(), frame.clone()).unwrap();
        sizes.insert(p.len());
        sa.send_to(&p, sb.local_addr().unwrap()).unwrap();
        let (len, _) = sb.recv_from(&mut buf).unwrap();
        let got = b.c.call_open(start.call_id.clone(), buf[..len].to_vec()).unwrap().unwrap();
        assert_eq!((got.counter, got.frame), (i as u64, frame));

        let r = b.c.call_seal(start.call_id.clone(), vec![0xAB; 60]).unwrap();
        sizes.insert(r.len());
        sb.send_to(&r, sa.local_addr().unwrap()).unwrap();
        let (len, _) = sa.recv_from(&mut buf).unwrap();
        assert_eq!(a.c.call_open(start.call_id.clone(), buf[..len].to_vec()).unwrap().unwrap().frame, vec![0xAB; 60]);
    }
    assert_eq!(sizes.len(), 1, "todos os pacotes tem o mesmo tamanho");

    // Pacote repetido, refletido ou adulterado: descartado.
    let p = a.c.call_seal(start.call_id.clone(), b"x".to_vec()).unwrap();
    assert!(b.c.call_open(start.call_id.clone(), p.clone()).unwrap().is_some());
    assert!(b.c.call_open(start.call_id.clone(), p.clone()).unwrap().is_none());
    assert!(a.c.call_open(start.call_id.clone(), p.clone()).unwrap().is_none());
    let mut t = a.c.call_seal(start.call_id.clone(), b"y".to_vec()).unwrap();
    t[30] ^= 1;
    assert!(b.c.call_open(start.call_id.clone(), t).unwrap().is_none());

    // Desligar: o Bob fica a saber, as chaves desaparecem dos dois lados (sem campainha).
    let bye = a.c.call_hangup(start.call_id.clone()).unwrap().unwrap();
    assert!(bye.silent && bye.wake.is_none());
    assert_eq!(
        a.events(),
        vec![CallEvent::Ended { call_id: start.call_id.clone(), contact_id: b.cid(), reason: CallEndReason::Hangup }]
    );
    relay(&a, &b);
    assert_eq!(
        b.events(),
        vec![CallEvent::Ended { call_id: start.call_id.clone(), contact_id: a.cid(), reason: CallEndReason::Hangup }]
    );
    assert!(a.c.call_seal(start.call_id.clone(), vec![]).is_err());
    assert!(b.c.call_open(start.call_id.clone(), p).is_err());
    assert_eq!((a.c.call_current().unwrap(), b.c.call_current().unwrap()), (None, None));
}

#[test]
fn recusar() {
    let (a, b) = pair();
    let start = a.c.call_start(b.cid()).unwrap();
    relay(&a, &b);
    b.events();
    let no = b.c.call_hangup(start.call_id.clone()).unwrap().unwrap();
    assert!(no.silent && no.wake.is_none(), "recusar: quem ligou tem a app aberta");
    relay(&b, &a);
    let ev = a.events();
    assert_eq!(
        ev.last(),
        Some(&CallEvent::Ended { call_id: start.call_id.clone(), contact_id: b.cid(), reason: CallEndReason::Rejected })
    );
    // Uma resposta atrasada a uma chamada que ja acabou e ignorada.
    assert!(b.c.call_accept(start.call_id).is_err());
}

#[test]
fn cancelar_antes_de_sair_nao_envia_nada() {
    let (a, b) = pair();
    let start = a.c.call_start(b.cid()).unwrap();
    // A oferta ainda nao foi ao relay: sai da outbox e nao se avisa ninguem.
    assert!(a.c.call_hangup(start.call_id.clone()).unwrap().is_none());
    assert_eq!(relay(&a, &b), (0, false));
    assert!(b.events().is_empty());
}

#[test]
fn cancelar_depois_de_tocar_e_chamada_perdida() {
    let (a, b) = pair();
    let start = a.c.call_start(b.cid()).unwrap();
    relay(&a, &b);
    b.events();
    // Cancelar antes de atender: a campainha marcada faz parar o toque do outro lado.
    let end = a.c.call_hangup(start.call_id.clone()).unwrap().unwrap();
    assert_eq!((end.silent, end.wake), (false, Some(CallWake::CallEnd)));
    relay(&a, &b);
    assert_eq!(
        b.events(),
        vec![CallEvent::Ended { call_id: start.call_id, contact_id: a.cid(), reason: CallEndReason::Cancelled }]
    );
}

#[test]
fn ocupado() {
    let (a, b) = pair();
    let c = peer("c", 0x83);
    connect(&c, &b);
    answered_call(&a, &b);

    let start = c.c.call_start(b.cid()).unwrap();
    relay(&c, &b);
    // O Bob nao ve o ecra de chamada: so fica registada como perdida (ocupado).
    assert_eq!(
        b.events(),
        vec![CallEvent::Ended { call_id: start.call_id.clone(), contact_id: c.cid(), reason: CallEndReason::Busy }]
    );
    relay(&b, &c);
    assert_eq!(
        c.events(),
        vec![CallEvent::Ended { call_id: start.call_id, contact_id: b.cid(), reason: CallEndReason::Busy }]
    );
    // A chamada com a Alice continua.
    assert!(b.c.call_current().unwrap().is_some());
}

#[test]
fn so_uma_chamada_de_cada_vez() {
    let (a, b) = pair();
    let c = peer("c", 0x84);
    connect(&a, &c);
    a.c.call_start(b.cid()).unwrap();
    assert!(a.c.call_start(c.cid()).is_err());
}

#[test]
fn ligar_os_dois_ao_mesmo_tempo_fica_uma_chamada() {
    let (a, b) = pair();
    let sa = a.c.call_start(b.cid()).unwrap();
    let sb = b.c.call_start(a.cid()).unwrap();
    // As duas ofertas cruzam-se no relay.
    let (oa, ob) = (a.c.outbox().unwrap(), b.c.outbox().unwrap());
    for i in oa {
        b.c.receive_sealed(i.blob).unwrap();
        a.c.outbox_sent(i.outbox_id).unwrap();
    }
    for i in ob {
        a.c.receive_sealed(i.blob).unwrap();
        b.c.outbox_sent(i.outbox_id).unwrap();
    }
    let winner = std::cmp::min(sa.call_id.clone(), sb.call_id.clone());
    assert_eq!(a.c.call_current().unwrap(), Some(winner.clone()));
    assert_eq!(b.c.call_current().unwrap(), Some(winner.clone()));
    // Quem recebe a chamada que fica pode atender; o audio funciona.
    let (callee, caller) = if winner == sa.call_id { (&b, &a) } else { (&a, &b) };
    assert!(callee.events().contains(&CallEvent::Incoming { call_id: winner.clone(), contact_id: caller.cid() }));
    callee.c.call_accept(winner.clone()).unwrap();
    relay(callee, caller);
    assert!(caller.events().contains(&CallEvent::Answered { call_id: winner.clone() }));
    let p = caller.c.call_seal(winner.clone(), b"ola".to_vec()).unwrap();
    assert_eq!(callee.c.call_open(winner, p).unwrap().unwrap().frame, b"ola".to_vec());
}

#[test]
fn contacto_bloqueado_nao_toca() {
    let (a, b) = pair();
    b.c.block_contact(a.cid()).unwrap();
    a.c.call_start(b.cid()).unwrap();
    relay(&a, &b);
    assert!(b.events().is_empty());
    assert!(b.c.call_current().unwrap().is_none());
    // E nao se consegue ligar a quem se bloqueou.
    assert!(b.c.call_start(a.cid()).is_err());
}

#[test]
fn a_mesma_oferta_entregue_duas_vezes_toca_uma_vez() {
    let (a, b) = pair();
    let start = a.c.call_start(b.cid()).unwrap();
    b.c.receive_sealed(start.item.blob.clone()).unwrap();
    // O relay reentrega (sem confirmacao): o ratchet recusa o repetido.
    assert!(b.c.receive_sealed(start.item.blob).is_err());
    assert_eq!(b.events().len(), 1);
}
