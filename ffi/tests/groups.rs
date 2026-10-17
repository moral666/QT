//! Grupos ate 10 pessoas (3 administradores), por envio a cada membro. Quatro
//! clientes reais; o "relay" e simulado: cada envelope da outbox e entregue ao
//! membro a quem se destina (e confirmado), tal como o relay faria.

use qt_ffi::groups::*;
use qt_ffi::history::*;
use qt_ffi::*;
use std::sync::Arc;

fn tmp(n: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_grp_{}_{}_{}.sqlite", n, std::process::id(), id);
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
    fn pid(&self) -> String {
        self.id.public_id.clone()
    }
    /// Abre sessao com `other` (como a app faz com o bundle do relay).
    fn session_with(&self, other: &Peer) {
        if !self.c.has_session(other.cid()).unwrap() {
            let bundle = peer_bundle_from_relay_bytes(other.c.publishable_bundle().unwrap()).unwrap();
            self.c.start_session(other.pid(), bundle).unwrap();
        }
    }
}

/// O relay: entrega cada envelope da outbox de `from` ao destinatario (se estiver em `all`).
fn route(from: &Peer, all: &[&Peer]) -> Vec<ReceiveResult> {
    let mut out = Vec::new();
    for item in from.c.outbox().unwrap() {
        if let Some(to) = all.iter().find(|p| p.cid() == item.contact_id) {
            out.push(to.c.receive_sealed(item.blob.clone()).unwrap());
        }
        from.c.outbox_sent(item.outbox_id).unwrap();
    }
    out
}

fn texts(p: &Peer, gid: &str) -> Vec<(String, String)> {
    p.c.group_messages_page(gid.to_string(), None, 50).unwrap().iter().rev().map(|m| (m.sender.clone(), m.text.clone())).collect()
}

/// Alice (administradora) cria o grupo com Bob e Carol, que tambem abrem sessao entre si.
fn setup() -> (Peer, Peer, Peer, String) {
    let (a, b, c) = (peer("alice", 0x11), peer("bob", 0x22), peer("carol", 0x33));
    a.session_with(&b);
    a.session_with(&c);
    let g = a.c.group_create("Família".into(), vec![b.pid(), c.pid()]).unwrap();
    assert!(g.i_am_admin && g.active && g.members.len() == 3);
    route(&a, &[&b, &c]);
    let gid = g.group_id.clone();
    // Bob e Carol precisam de sessao entre si para escrever no grupo.
    assert_eq!(b.c.group_members_needing_session(gid.clone()).unwrap(), vec![c.pid()]);
    b.session_with(&c);
    c.session_with(&b);
    (a, b, c, gid)
}

#[test]
fn criar_escrever_e_receber_com_quem_escreveu() {
    let (a, b, c, gid) = setup();
    for p in [&b, &c] {
        let g = p.c.group(gid.clone()).unwrap().expect("o grupo chegou");
        assert_eq!(g.name, "Família");
        assert!(g.active && !g.i_am_admin);
        assert!(g.members.iter().any(|m| m.me));
    }

    let m = a.c.group_send_text(gid.clone(), "olá a todos".into()).unwrap();
    assert_eq!(m.status, MessageStatus::Pending);
    assert_eq!(m.contact_id, format!("g:{gid}"));
    route(&a, &[&b, &c]);
    assert_eq!(a.c.group_messages_page(gid.clone(), None, 10).unwrap()[0].status, MessageStatus::Sent, "o relay confirmou os dois envelopes");
    for p in [&b, &c] {
        assert_eq!(texts(p, &gid), vec![(a.cid(), "olá a todos".to_string())]);
    }

    // O Bob responde (a Carol recebe tambem o convite do Bob, que nao aparece).
    b.c.group_send_text(gid.clone(), "olá!".into()).unwrap();
    route(&b, &[&a, &c]);
    assert_eq!(texts(&c, &gid).last().unwrap(), &(b.cid(), "olá!".to_string()));
    assert_eq!(texts(&a, &gid).last().unwrap(), &(b.cid(), "olá!".to_string()));
    // Estar no mesmo grupo nao cria um convite "Quer falar contigo".
    assert!(c.c.pending_intros().unwrap().is_empty());
    // A conversa aparece na lista com o id "g:...".
    assert!(a.c.conversations().unwrap().iter().any(|s| s.contact_id == format!("g:{gid}")));
}

#[test]
fn reacoes_no_grupo() {
    let (a, b, c, gid) = setup();
    let m = a.c.group_send_text(gid.clone(), "jantar às 8?".into()).unwrap();
    route(&a, &[&b, &c]);
    c.c.group_react(gid.clone(), m.message_id.clone(), "👍".into()).unwrap();
    let r = route(&c, &[&a, &b]);
    // (Ao Bob vai tambem o convite da Carol, que e outro envelope.)
    let updated = r.iter().filter(|x| matches!(x, ReceiveResult::Updated { contact_id } if *contact_id == format!("g:{gid}"))).count();
    assert_eq!(updated, 2);
    for p in [&a, &b] {
        let got = &p.c.group_messages_page(gid.clone(), None, 10).unwrap()[0].reactions;
        assert_eq!(got, &vec![Reaction { emoji: "👍".into(), mine: false, from_contact: c.cid() }]);
    }
}

#[test]
fn so_administradores_mudam_o_grupo_e_limites() {
    let (a, b, c, gid) = setup();
    let members = |admins: &[bool]| -> Vec<GroupMemberInput> {
        [&a, &b, &c].iter().zip(admins).map(|(p, ad)| GroupMemberInput { public_id: p.pid(), admin: *ad }).collect()
    };
    // O Bob nao e administrador: nao muda nada.
    assert!(b.c.group_update(gid.clone(), "Outro".into(), members(&[true, true, false])).is_err());

    // A Alice promove o Bob e muda o nome; os dois outros recebem a versao 2.
    let g = a.c.group_update(gid.clone(), "Família 🎉".into(), members(&[true, true, false])).unwrap();
    assert_eq!(g.version, 2);
    route(&a, &[&b, &c]);
    assert!(b.c.group(gid.clone()).unwrap().unwrap().i_am_admin);
    assert_eq!(c.c.group(gid.clone()).unwrap().unwrap().name, "Família 🎉");

    // No maximo 3 administradores e 10 membros.
    let extra: Vec<Peer> = (0..8).map(|i| peer("x", 0x40 + i)).collect();
    let mut big: Vec<GroupMemberInput> = members(&[true, true, true]);
    big.push(GroupMemberInput { public_id: extra[0].pid(), admin: true });
    assert!(a.c.group_update(gid.clone(), "x".into(), big).is_err(), "4 administradores");
    let mut eleven = members(&[true, false, false]);
    for p in &extra {
        eleven.push(GroupMemberInput { public_id: p.pid(), admin: false });
    }
    assert_eq!(eleven.len(), 11);
    assert!(a.c.group_update(gid.clone(), "x".into(), eleven).is_err(), "11 membros");
    assert!(a.c.group_create("x".into(), extra.iter().map(|p| p.pid()).chain([b.pid(), c.pid()]).collect()).is_err());
}

#[test]
fn remover_um_membro_e_ele_fica_sem_o_grupo() {
    let (a, b, c, gid) = setup();
    let g = a.c.group_update(
        gid.clone(),
        "Família".into(),
        vec![GroupMemberInput { public_id: a.pid(), admin: true }, GroupMemberInput { public_id: b.pid(), admin: false }],
    )
    .unwrap();
    assert_eq!(g.members.len(), 2);
    route(&a, &[&b, &c]);
    let cg = c.c.group(gid.clone()).unwrap().unwrap();
    assert!(!cg.active, "a Carol foi removida");
    assert!(c.c.group_send_text(gid.clone(), "ainda aqui?".into()).is_err());
    assert_eq!(b.c.group(gid.clone()).unwrap().unwrap().members.len(), 2);
}

#[test]
fn sair_do_grupo_e_o_admin_publica_versao_nova() {
    let (a, b, c, gid) = setup();
    b.c.group_leave(gid.clone()).unwrap();
    assert!(!b.c.group(gid.clone()).unwrap().unwrap().active);
    route(&b, &[&a, &c]);
    // A Alice (administradora) tirou o Bob e mandou a versao nova a Carol.
    let ag = a.c.group(gid.clone()).unwrap().unwrap();
    assert_eq!((ag.members.len(), ag.version), (2, 2));
    route(&a, &[&b, &c]);
    let cg = c.c.group(gid.clone()).unwrap().unwrap();
    assert_eq!((cg.members.len(), cg.version), (2, 2));

    // A administradora sai: a Carol (unico membro) passa a administradora.
    a.c.group_leave(gid.clone()).unwrap();
    route(&a, &[&c]);
    let cg = c.c.group(gid.clone()).unwrap().unwrap();
    assert!(cg.i_am_admin && cg.members.len() == 1);

    // Apagar o grupo deste telemovel.
    a.c.group_delete(gid.clone()).unwrap();
    assert!(a.c.group(gid.clone()).unwrap().is_none());
    assert!(a.c.group_messages_page(gid, None, 10).unwrap().is_empty());
}

#[test]
fn mensagem_que_chega_antes_da_info_do_grupo_nao_se_perde() {
    let (a, b, c) = (peer("alice", 0x51), peer("bob", 0x52), peer("carol", 0x53));
    a.session_with(&b);
    a.session_with(&c);
    let gid = a.c.group_create("G".into(), vec![b.pid(), c.pid()]).unwrap().group_id;
    // A info so chega ao Bob; a da Carol fica "no relay".
    let held: Vec<_> = a.c.outbox().unwrap().into_iter().filter(|i| i.contact_id == c.cid()).collect();
    for item in a.c.outbox().unwrap() {
        if item.contact_id == b.cid() {
            b.c.receive_sealed(item.blob.clone()).unwrap();
        }
        a.c.outbox_sent(item.outbox_id).unwrap();
    }
    b.session_with(&c);
    b.c.group_send_text(gid.clone(), "cheguei primeiro".into()).unwrap();
    route(&b, &[&a, &c]);
    assert!(c.c.group(gid.clone()).unwrap().is_none(), "a Carol ainda nao conhece o grupo");
    // Chega a info: a mensagem guardada entra no historico.
    for item in held {
        c.c.receive_sealed(item.blob).unwrap();
    }
    assert_eq!(texts(&c, &gid), vec![(b.cid(), "cheguei primeiro".to_string())]);
}

#[test]
fn quem_esta_bloqueado_nao_entra_no_grupo_de_quem_bloqueou() {
    let (a, b, c, gid) = setup();
    c.c.block_contact(b.cid()).unwrap();
    b.c.group_send_text(gid.clone(), "olá".into()).unwrap();
    route(&b, &[&a, &c]);
    assert!(texts(&c, &gid).is_empty(), "a Carol bloqueou o Bob");
    assert_eq!(texts(&a, &gid).len(), 1);
    // E a Carol nao lhe envia: so a Alice recebe.
    c.c.group_send_text(gid.clone(), "só para a Alice".into()).unwrap();
    assert!(c.c.outbox().unwrap().iter().all(|i| i.contact_id != b.cid()));
}

/// Dois membros que nao se conheciam abrem sessao um com o outro ao mesmo tempo
/// (sessoes cruzadas) e escrevem logo: nada se perde, nos dois lados do desempate.
#[test]
fn dois_membros_novos_escrevem_ao_mesmo_tempo() {
    for ronda in 0..6 {
        let (a, b, c, gid) = setup();
        b.c.group_send_text(gid.clone(), "do Bob".into()).unwrap();
        c.c.group_send_text(gid.clone(), "da Carol".into()).unwrap();
        for _ in 0..3 {
            route(&b, &[&a, &c]);
            route(&c, &[&a, &b]);
        }
        let has = |p: &Peer, t: &str| texts(p, &gid).iter().any(|(_, x)| x == t);
        assert!(has(&c, "do Bob") && has(&b, "da Carol"), "ronda {ronda}");
        assert!(has(&a, "do Bob") && has(&a, "da Carol"), "ronda {ronda}");
        assert_eq!(texts(&b, &gid).len(), 2, "ronda {ronda}: sem duplicados");
        // E continua nos dois sentidos, ja com uma so sessao.
        b.c.group_send_text(gid.clone(), "segunda".into()).unwrap();
        c.c.group_send_text(gid.clone(), "resposta".into()).unwrap();
        route(&b, &[&a, &c]);
        route(&c, &[&a, &b]);
        assert!(has(&c, "segunda") && has(&b, "resposta"), "ronda {ronda}");
        for p in [&b, &c] {
            for m in p.c.group_messages_page(gid.clone(), None, 20).unwrap().iter().filter(|m| m.outgoing) {
                assert_eq!(m.status, MessageStatus::Sent, "ronda {ronda}: {}", m.text);
            }
        }
    }
}

#[test]
fn criar_sem_ninguem_e_sem_nome() {
    let a = peer("alice", 0x61);
    let g = a.c.group_create("   ".into(), vec![]).unwrap();
    assert_eq!((g.name.as_str(), g.members.len(), g.i_am_admin), ("Grupo", 1, true));
    assert!(a.c.outbox().unwrap().is_empty());
    // Escrever num grupo so comigo: fica "Enviada" logo (nao ha a quem entregar).
    let m = a.c.group_send_text(g.group_id.clone(), "nota".into()).unwrap();
    assert_eq!(m.status, MessageStatus::Sent);
    // Nome comprido ou com mudancas de linha: cortado e numa so linha, nunca recusado.
    let long = "á".repeat(80);
    let g2 = a.c.group_create(format!("linha 1\nlinha 2 {long}"), vec![]).unwrap();
    assert!(g2.name.starts_with("linha 1 linha 2") && g2.name.len() <= 100);
    let r = a.c.group_update(g.group_id.clone(), "".into(), vec![GroupMemberInput { public_id: a.id.public_id.clone(), admin: true }]).unwrap();
    assert_eq!(r.name, "Grupo");
}

#[test]
fn id_da_conversa_do_grupo() {
    assert_eq!(group_id_of_conversation("g:00ff".into()), Some("00ff".into()));
    assert_eq!(group_id_of_conversation("00ff".into()), None);
}
