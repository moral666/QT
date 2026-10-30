//! Comunidade (0.9.52): o envio da foto de quem tem medalha (so quando e preciso) e a lista
//! guardada na app (assinatura, numero, fotos so as novas, ordem).

use base64::Engine;
use qt_core::badge::{issue, issuer_public, BADGE_DEV, BADGE_FOUNDER, BADGE_LEGACY, BADGE_NONE, BADGE_TROLL};
use qt_core::{community as qc, report};
use qt_ffi::*;
use std::sync::Arc;

const ISSUER: [u8; 32] = [0x6a; 32];
const OPERATOR: [u8; 32] = [0x6b; 32];

fn tmp(n: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_comunidade_{}_{}_{}.sqlite", n, std::process::id(), id);
    let _ = std::fs::remove_file(&p);
    p
}

fn setup() {
    qt_ffi::badges::set_test_issuer(issuer_public(&ISSUER));
    qt_ffi::community::set_test_operator(report::operator_report_public(&OPERATOR));
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

/// O emissor (como a ferramenta `badge`) entrega um certificado a `to`.
fn grant(issuer: &Peer, to: &Peer, badge: u8, at: i64) {
    if !issuer.c.has_session(to.cid()).unwrap() {
        let bundle = peer_bundle_from_relay_bytes(to.c.publishable_bundle().unwrap()).unwrap();
        issuer.c.start_session(to.id.public_id.clone(), bundle).unwrap();
    }
    let subject: [u8; 32] = to.id.identity_public.clone().try_into().unwrap();
    issuer.c.badge_deliver(to.cid(), issue(&ISSUER, &subject, badge, at)).unwrap();
    for item in issuer.c.outbox().unwrap() {
        if item.contact_id == to.cid() {
            to.c.receive_sealed(item.blob.clone()).unwrap();
        }
        issuer.c.outbox_sent(item.outbox_id).unwrap();
    }
}

fn webp(n: usize, fill: u8) -> Vec<u8> {
    let mut p = [b"RIFF".as_slice(), &[0, 0, 0, 0], b"WEBPVP8 "].concat();
    p.resize(n, fill);
    p
}

/// O que o relay guarda (o `sealed` do pedido), aberto pelo operador.
fn open(req: &[u8]) -> qc::Submission {
    let v: serde_json::Value = serde_json::from_slice(req).unwrap();
    let sealed = base64::engine::general_purpose::STANDARD.decode(v["CommunitySubmit"]["sealed"].as_str().unwrap()).unwrap();
    qc::open_submission(&OPERATOR, &issuer_public(&ISSUER), &sealed).unwrap()
}

#[test]
fn envio_so_com_medalha_e_so_quando_muda() {
    setup();
    let a = peer("a", 0x61);
    let nuno = peer("emissor", 0x62);
    // Sem medalha: nada.
    assert_eq!(a.c.community_submit_request().unwrap(), None);

    grant(&nuno, &a, BADGE_DEV, 1_000);
    let req = a.c.community_submit_request().unwrap().expect("primeiro envio");
    let s = open(&req);
    assert_eq!((s.cert.badge, s.photo.clone()), (BADGE_DEV, None));
    assert_eq!(s.cert.subject.to_vec(), a.id.identity_public);
    // Ainda sem o Ack: continua a pedir. Depois do Ack: nada ate mudar.
    assert!(a.c.community_submit_request().unwrap().is_some());
    a.c.community_submit_sent().unwrap();
    assert_eq!(a.c.community_submit_request().unwrap(), None);

    // Foto nova: envia outra vez, com a foto (a mesma miniatura dos contactos).
    let photo = webp(9_973, 4);
    a.c.profile_photo_set(photo.clone(), vec![]).unwrap();
    let req = a.c.community_submit_request().unwrap().expect("foto nova");
    assert_eq!(open(&req).photo, Some(photo));
    a.c.community_submit_sent().unwrap();
    assert_eq!(a.c.community_submit_request().unwrap(), None);

    // Tirar a foto tambem conta.
    a.c.profile_photo_remove(vec![]).unwrap();
    let req = a.c.community_submit_request().unwrap().expect("sem foto");
    assert_eq!(open(&req).photo, None);
    a.c.community_submit_sent().unwrap();

    // Medalha nova: envia; medalha retirada: deixa de enviar.
    grant(&nuno, &a, BADGE_FOUNDER, 2_000);
    assert_eq!(open(&a.c.community_submit_request().unwrap().unwrap()).cert.badge, BADGE_FOUNDER);
    grant(&nuno, &a, BADGE_NONE, 3_000);
    assert_eq!(a.c.community_submit_request().unwrap(), None);
}

fn member(badge: u8, h: Option<[u8; 32]>) -> qc::Member {
    qc::Member { badge, photo_hash: h }
}

#[test]
fn lista_assinada_guardada_com_so_as_fotos_novas() {
    setup();
    let b = peer("b", 0x63);
    assert_eq!(b.c.community_cached_version().unwrap(), 0);
    assert!(b.c.community_members().unwrap().is_empty());

    let (h1, f1) = qc::seal_photo(&webp(500, 1)).unwrap();
    let (h2, f2) = qc::seal_photo(&webp(600, 2)).unwrap();
    let hex = |h: &[u8; 32]| h.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let ix = qc::Index {
        version: 10,
        created_ms: 1,
        members: vec![member(BADGE_TROLL, Some(h1)), member(BADGE_LEGACY, None), member(BADGE_FOUNDER, Some(h2))],
    };
    let blob = qc::build_index(&ISSUER, &ix).unwrap();
    // Numero diferente do da lista: recusado.
    assert!(b.c.community_apply_index(11, blob.clone()).is_err());
    let missing = b.c.community_apply_index(10, blob.clone()).unwrap();
    assert_eq!(missing, vec![hex(&h1), hex(&h2)]);
    assert_eq!(b.c.community_cached_version().unwrap(), 10);
    // Foto com o hash de outra: nao guardada. As certas: guardadas.
    assert!(!b.c.community_put_photo(hex(&h1), f2.clone()).unwrap());
    assert!(b.c.community_put_photo(hex(&h1), f1).unwrap());
    assert_eq!(b.c.community_missing_photos().unwrap(), vec![hex(&h2)]);
    assert!(b.c.community_put_photo(hex(&h2), f2).unwrap());
    assert!(b.c.community_missing_photos().unwrap().is_empty());

    // Pela ordem das medalhas, so medalha e foto (sem nome nem ID).
    let got = b.c.community_members().unwrap();
    let shown: Vec<_> = got.iter().map(|m| (m.badge, m.photo.as_ref().map(|p| p.len()))).collect();
    assert_eq!(shown, [(BADGE_FOUNDER, Some(600)), (BADGE_TROLL, Some(500)), (BADGE_LEGACY, None)]);

    // Lista seguinte: o Troll saiu, entra um Dev com foto nova.
    let (h3, f3) = qc::seal_photo(&webp(700, 3)).unwrap();
    let ix2 = qc::Index {
        version: 11,
        created_ms: 2,
        members: vec![member(BADGE_FOUNDER, Some(h2)), member(BADGE_DEV, Some(h3))],
    };
    let blob2 = qc::build_index(&ISSUER, &ix2).unwrap();
    // So desce a foto nova; a do Troll e apagada.
    assert_eq!(b.c.community_apply_index(11, blob2).unwrap(), vec![hex(&h3)]);
    assert!(b.c.community_put_photo(hex(&h3), f3).unwrap());
    // Uma foto que nao esta na lista nao e aceite.
    let (_, f1b) = qc::seal_photo(&webp(500, 1)).unwrap();
    assert!(!b.c.community_put_photo(hex(&qc::photo_hash(&f1b)), f1b).unwrap());
    assert_eq!(b.c.community_members().unwrap().len(), 2);

    // Uma lista antiga (numero mais baixo) nunca substitui a atual.
    assert!(b.c.community_apply_index(10, blob).is_err());
    assert_eq!(b.c.community_cached_version().unwrap(), 11);

    // Assinada por outra chave: recusada.
    let fake = qc::build_index(&[9u8; 32], &qc::Index { version: 12, ..ix2 }).unwrap();
    assert!(b.c.community_apply_index(12, fake).is_err());
}

#[test]
fn respostas_do_relay_da_comunidade() {
    let r = relay_parse_response(br#"{"CommunityVersion":{"version":7}}"#.to_vec()).unwrap();
    assert!(matches!(r, RelayResponse::CommunityVersion { version: 7 }));
    let r = relay_parse_response(br#""CommunityNone""#.to_vec()).unwrap();
    assert!(matches!(r, RelayResponse::CommunityNone));
    let r = relay_parse_response(br#"{"CommunityPhotos":{"photos":[{"hash":"ab","data":"AQI="}]}}"#.to_vec()).unwrap();
    assert!(matches!(r, RelayResponse::CommunityPhotos { ref photos } if photos[0].data == vec![1, 2]));
    let c = QtClient::open(tmp("p"), vec![0x64; 32]).unwrap();
    assert!(c.relay_request_community_photos(vec!["x".into()]).is_err());
    assert!(c.relay_request_community_photos(vec!["ab".repeat(32); 21]).is_err());
    assert_eq!(c.relay_request_community_version().unwrap(), br#""CommunityVersion""#);
}
