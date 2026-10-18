//! Sessoes cruzadas, reinicio de sessao e eliminacao segura de contactos/conversas.
//!
//! Cenarios reais que faziam mensagens desaparecer SEM erro:
//!  * as duas pessoas adicionam-se e escrevem ao mesmo tempo (cada uma inicia
//!    a sua sessao PQXDH; nenhuma conseguia ler a da outra);
//!  * alguem apaga o contacto (esquece a sessao) e volta a adiciona-lo.

use qt_ffi::history::*;
use qt_ffi::*;
use std::sync::Arc;

fn tmp(n: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_reset_{}_{}_{}.sqlite", n, std::process::id(), id);
    let _ = std::fs::remove_file(&p);
    p
}

struct Peer {
    path: String,
    c: Arc<QtClient>,
    id: PublicIdentity,
}

fn peer(name: &str, key: u8) -> Peer {
    let path = tmp(name);
    let c = QtClient::open(path.clone(), vec![key; 32]).unwrap();
    let id = c.create_identity().unwrap();
    Peer { path, c, id }
}

impl Peer {
    fn cid(&self) -> String {
        self.id.identity_public.iter().map(|b| format!("{b:02x}")).collect()
    }
    fn start_with(&self, other: &Peer) {
        let bundle = peer_bundle_from_relay_bytes(other.c.publishable_bundle().unwrap()).unwrap();
        self.c.start_session(other.id.public_id.clone(), bundle).unwrap();
    }
    fn texts(&self, contact: &str) -> Vec<String> {
        let mut v: Vec<String> =
            self.c.messages_page(contact.to_string(), None, 500).unwrap().into_iter().map(|m| m.text).collect();
        v.reverse();
        v
    }
}

/// Relay simulado: entrega a outbox de `from` a `to` (erros definitivos sao
/// confirmados e descartados, como faz a app) e marca tudo como enviado.
fn relay(from: &Peer, to: &Peer) -> usize {
    let mut ok = 0;
    for item in from.c.outbox().unwrap() {
        if to.c.receive_sealed(item.blob.clone()).is_ok() {
            ok += 1;
        }
        from.c.outbox_sent(item.outbox_id).unwrap();
    }
    ok
}

/// Varias voltas de relay nos dois sentidos (recibos, reenvios...).
fn settle(a: &Peer, b: &Peer) {
    for _ in 0..4 {
        a.c.prepare_delivery_receipts().unwrap();
        b.c.prepare_delivery_receipts().unwrap();
        relay(a, b);
        relay(b, a);
    }
}

#[test]
fn sessoes_cruzadas_nao_perdem_mensagens() {
    let a = peer("a", 0x31);
    let b = peer("b", 0x32);
    // Os dois adicionam-se e escrevem antes de receberem o que quer que seja.
    a.start_with(&b);
    b.start_with(&a);
    a.c.send_text(b.cid(), "olá da Alice".into()).unwrap();
    b.c.send_text(a.cid(), "olá do Bob".into()).unwrap();

    settle(&a, &b);

    for t in ["olá da Alice", "olá do Bob"] {
        assert!(a.texts(&b.cid()).contains(&t.to_string()), "Alice nao tem '{t}': {:?}", a.texts(&b.cid()));
        assert!(b.texts(&a.cid()).contains(&t.to_string()), "Bob nao tem '{t}': {:?}", b.texts(&a.cid()));
    }

    // E a conversa continua nos dois sentidos.
    a.c.send_text(b.cid(), "segunda".into()).unwrap();
    b.c.send_text(a.cid(), "resposta".into()).unwrap();
    settle(&a, &b);
    assert!(b.texts(&a.cid()).contains(&"segunda".to_string()));
    assert!(a.texts(&b.cid()).contains(&"resposta".to_string()));

    // Todas as mensagens enviadas acabam "Entregue" (ou "Visto").
    for m in a.c.messages_page(b.cid(), None, 100).unwrap().into_iter().filter(|m| m.outgoing) {
        assert!(matches!(m.status, MessageStatus::Delivered | MessageStatus::Read), "{}: {:?}", m.text, m.status);
    }
    let _ = (&a.path, &b.path);
}

#[test]
fn sessoes_cruzadas_funcionam_nos_dois_sentidos_do_desempate() {
    // As identidades sao aleatorias: em varias tentativas, o desempate cai
    // umas vezes para cada lado.
    for i in 0..6u8 {
        let a = peer("xa", 0x40 + i);
        let b = peer("xb", 0x50 + i);
        a.start_with(&b);
        b.start_with(&a);
        a.c.send_text(b.cid(), "A1".into()).unwrap();
        a.c.send_text(b.cid(), "A2".into()).unwrap();
        b.c.send_text(a.cid(), "B1".into()).unwrap();
        settle(&a, &b);
        for t in ["A1", "A2", "B1"] {
            assert!(a.texts(&b.cid()).contains(&t.to_string()), "ronda {i}: Alice sem {t}");
            assert!(b.texts(&a.cid()).contains(&t.to_string()), "ronda {i}: Bob sem {t}");
        }
        // Sem duplicados.
        assert_eq!(b.texts(&a.cid()).len(), 3, "ronda {i}: {:?}", b.texts(&a.cid()));
    }
}

#[test]
fn eliminar_conversa_apaga_mensagens_mas_a_conversa_continua() {
    let a = peer("ca", 0x61);
    let b = peer("cb", 0x62);
    a.start_with(&b);
    a.c.send_text(b.cid(), "um".into()).unwrap();
    settle(&a, &b);
    b.c.send_text(a.cid(), "dois".into()).unwrap();
    settle(&a, &b);
    assert_eq!(a.texts(&b.cid()).len(), 2);

    a.c.delete_conversation(b.cid()).unwrap();
    assert!(a.texts(&b.cid()).is_empty());
    assert!(a.c.conversations().unwrap().is_empty());
    assert!(a.c.has_session(b.cid()).unwrap(), "a sessao cifrada mantem-se");

    b.c.send_text(a.cid(), "tres".into()).unwrap();
    a.c.send_text(b.cid(), "quatro".into()).unwrap();
    settle(&a, &b);
    assert_eq!(a.texts(&b.cid()).len(), 2, "so as mensagens depois de eliminar: {:?}", a.texts(&b.cid()));
    assert!(a.texts(&b.cid()).contains(&"tres".to_string()));
    assert!(b.texts(&a.cid()).contains(&"quatro".to_string()));
}

#[test]
fn apagar_contacto_apaga_tudo_e_voltar_a_adicionar_funciona() {
    let a = peer("da", 0x71);
    let b = peer("db", 0x72);
    a.start_with(&b);
    let first = a.c.outbox().unwrap();
    a.c.send_text(b.cid(), "antes".into()).unwrap();
    let old_blobs: Vec<Vec<u8>> = a.c.outbox().unwrap().into_iter().map(|i| i.blob).collect();
    assert!(first.is_empty());
    settle(&a, &b);
    b.c.send_text(a.cid(), "resposta".into()).unwrap();
    settle(&a, &b);

    // Bob apaga a Alice: mensagens, sessao e metadados desaparecem.
    b.c.delete_contact(a.cid()).unwrap();
    assert!(b.texts(&a.cid()).is_empty());
    assert!(!b.c.has_session(a.cid()).unwrap());
    assert!(b.c.conversations().unwrap().is_empty());
    assert!(b.c.outbox().unwrap().is_empty());

    // O relay reentrega uma mensagem antiga da Alice: e reconhecida como
    // repetida e NAO recria a sessao nem a conversa.
    for blob in &old_blobs {
        assert!(b.c.receive_sealed(blob.clone()).is_err());
    }
    assert!(!b.c.has_session(a.cid()).unwrap());
    assert!(b.texts(&a.cid()).is_empty());

    // Nenhum registo com o contacto em claro fica na base de dados.
    drop(b.c);
    let b = Peer { path: b.path.clone(), c: QtClient::open(b.path.clone(), vec![0x72; 32]).unwrap(), id: b.id };
    let raw = std::fs::read(&b.path).unwrap();
    assert!(!raw.windows(8).any(|w| w == b"resposta"), "o ficheiro esta cifrado e nao tem texto em claro");

    // Bob volta a adicionar a Alice e escreve: a Alice adota a sessao nova e
    // a conversa continua nos dois sentidos.
    b.start_with(&a);
    b.c.send_text(a.cid(), "voltei".into()).unwrap();
    settle(&a, &b);
    assert!(a.texts(&b.cid()).contains(&"voltei".to_string()), "{:?}", a.texts(&b.cid()));
    a.c.send_text(b.cid(), "bem-vindo".into()).unwrap();
    settle(&a, &b);
    assert!(b.texts(&a.cid()).contains(&"bem-vindo".to_string()), "{:?}", b.texts(&a.cid()));
}

#[test]
fn apagar_contacto_valida_o_id() {
    let a = peer("va", 0x75);
    assert!(a.c.delete_contact("nao-e-hex".into()).is_err());
    assert!(a.c.delete_conversation("".into()).is_err());
}

/// Imagem enviada durante sessoes cruzadas: a mensagem com a chave do anexo tambem
/// e cifrada de novo com a sessao que fica (nos dois sentidos do desempate).
#[test]
fn sessoes_cruzadas_nao_perdem_anexos() {
    // Fica a sessao de quem tem a chave de identidade menor. Forca os dois casos:
    // a Alice perde o desempate (tem de cifrar de novo o anexo) e depois ganha.
    for alice_loses in [true, false] {
        let (a, b) = loop {
            let a = peer("att_a", 0x41);
            let b = peer("att_b", 0x42);
            if (a.id.identity_public > b.id.identity_public) == alice_loses {
                break (a, b);
            }
        };
        a.start_with(&b);
        b.start_with(&a);
        let src = format!("{}.src", a.path);
        std::fs::write(&src, vec![7u8; 5000]).unwrap();
        let m = a.c.attachment_create(b.cid(), src.clone(), "image/png".into(), 50, 40).unwrap();
        a.c.attachment_set_blob(b.cid(), m.message_id.clone(), "ab".repeat(16), "cd".repeat(16)).unwrap();
        a.c.attachment_uploaded(b.cid(), m.message_id.clone()).unwrap();
        b.c.send_text(a.cid(), "olá do Bob".into()).unwrap();
        settle(&a, &b);
        let got = b.c.messages_page(a.cid(), None, 50).unwrap();
        let att = got.iter().find(|x| x.message_id == m.message_id).expect("o anexo chegou");
        let info = att.attachment.clone().unwrap();
        assert_eq!((info.width, info.height, info.size), (50, 40, 5000));
        assert_eq!(b.c.attachments_pending_download().unwrap().len(), 1, "fica a espera de descarregar");
        assert!(a.texts(&b.cid()).contains(&"olá do Bob".to_string()));
        let _ = std::fs::remove_file(src);
    }
}

#[test]
fn recibos_convites_e_presenca_vao_sem_campainha() {
    let a = peer("sa", 0x77);
    let b = peer("sb", 0x78);
    a.start_with(&b);
    a.c.send_text(b.cid(), "olá".into()).unwrap();
    // Convite (o meu Public ID) primeiro, sem campainha; depois a mensagem, com campainha.
    let out = a.c.outbox().unwrap();
    assert_eq!(out.iter().map(|i| i.silent).collect::<Vec<_>>(), vec![true, false]);
    assert!(out.iter().all(|i| i.relay_id.is_none()));
    settle(&a, &b);
    // O recibo "entregue" do Bob tambem vai sem campainha.
    b.c.send_text(a.cid(), "resposta".into()).unwrap();
    b.c.prepare_delivery_receipts().unwrap();
    let flags: Vec<bool> = b.c.outbox().unwrap().iter().map(|i| i.silent).collect();
    assert!(flags.contains(&false), "a mensagem toca: {flags:?}");
    let req = String::from_utf8(b.c.relay_request_send_item(a.c.relay_user_id().unwrap(), b.c.outbox().unwrap()[0].clone()).unwrap()).unwrap();
    assert!(req.contains("\"silent\""), "{req}");
}

#[test]
fn apagar_contacto_avisa_e_o_outro_volta_a_conseguir_escrever() {
    let a = peer("ea", 0x81);
    let b = peer("eb", 0x82);
    a.start_with(&b);
    a.c.send_text(b.cid(), "olá".into()).unwrap();
    settle(&a, &b);
    b.c.send_text(a.cid(), "resposta".into()).unwrap();
    settle(&a, &b);

    // A Alice apaga o Bob e avisa (o aviso vai para o endereco dele, que ja nao esta nos contactos).
    let b_relay = b.c.relay_user_id().unwrap();
    a.c.delete_contact_notify(b.cid(), b_relay.clone()).unwrap();
    assert!(!a.c.has_session(b.cid()).unwrap());
    assert!(a.texts(&b.cid()).is_empty());
    let out = a.c.outbox().unwrap();
    assert_eq!(out.len(), 1, "so o aviso");
    assert!(out[0].silent);
    assert_eq!(out[0].relay_id.as_deref(), Some(b_relay.as_str()));

    // O Bob escreve antes de o aviso chegar: a Alice nao consegue ler (sem sessao).
    b.c.send_text(a.cid(), "perdida".into()).unwrap();
    relay(&b, &a);
    assert!(a.texts(&b.cid()).is_empty());

    // O aviso chega: o Bob apaga a sessao, mas fica com o contacto e o historico.
    relay(&a, &b);
    assert!(a.c.outbox().unwrap().is_empty());
    assert!(!b.c.has_session(a.cid()).unwrap());
    assert_eq!(b.c.sessions_to_restart().unwrap(), vec![a.cid()]);
    assert_eq!(b.texts(&a.cid()).len(), 3, "{:?}", b.texts(&a.cid()));
    assert!(b.c.outbox().unwrap().is_empty(), "o que foi cifrado com a sessao antiga fica retido");

    // A app do Bob abre uma sessao nova: a mensagem perdida volta a seguir, com o convite.
    b.start_with(&a);
    assert!(b.c.sessions_to_restart().unwrap().is_empty());
    settle(&a, &b);
    assert!(a.texts(&b.cid()).contains(&"perdida".to_string()), "{:?}", a.texts(&b.cid()));
    assert_eq!(a.c.pending_intro(b.cid()).unwrap(), Some(b.id.public_id.clone()), "aparece como convite");
    assert_eq!(a.texts(&b.cid()).len(), 1, "sem duplicados: {:?}", a.texts(&b.cid()));

    // E a conversa continua nos dois sentidos.
    b.c.send_text(a.cid(), "outra".into()).unwrap();
    settle(&a, &b);
    assert!(a.texts(&b.cid()).contains(&"outra".to_string()));
    a.c.send_text(b.cid(), "aceite".into()).unwrap();
    settle(&a, &b);
    assert!(b.texts(&a.cid()).contains(&"aceite".to_string()));
}

#[test]
fn apagar_contacto_bloqueado_nao_avisa() {
    let a = peer("fa", 0x83);
    let b = peer("fb", 0x84);
    a.start_with(&b);
    a.c.send_text(b.cid(), "olá".into()).unwrap();
    settle(&a, &b);
    a.c.block_contact(b.cid()).unwrap();
    a.c.delete_contact_notify(b.cid(), b.c.relay_user_id().unwrap()).unwrap();
    assert!(a.c.outbox().unwrap().is_empty());
    assert!(a.c.delete_contact_notify(b.cid(), "x".into()).is_err());
}
