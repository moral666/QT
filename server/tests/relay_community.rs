//! Comunidade (0.9.52): envios selados das apps, lidos e apagados so pelo operador, e a lista
//! (indice e fotos) publicada pelo operador e lida por qualquer conta. Precisa de um Redis em
//! 127.0.0.1:6379.

mod common;
use common::*;

use base64::Engine;
use qt_core::primitives::SigningKeyPair;
use qt_core::{badge, community, report};
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::protocol::*;
use qt_server::ServerConfig;
use qt_transport::generate_static_keypair;
use x25519_dalek::{PublicKey, StaticSecret};

const SEED: [u8; 32] = [42u8; 32];
const ISSUER: [u8; 32] = [43u8; 32];

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn unb64(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

fn uid(s: &SigningKeyPair) -> String {
    user_id_from_signing_public(&s.verifying_key.to_bytes())
}

fn operator() -> SigningKeyPair {
    SigningKeyPair::from_bytes(&report::operator_signing_private(&SEED))
}

fn cfg() -> ServerConfig {
    ServerConfig { operator: Some(uid(&operator())), ..ServerConfig::default() }
}

async fn client(relay: &TestRelay, s: &SigningKeyPair) -> RelayClient {
    let n = generate_static_keypair().unwrap();
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    RelayClient::connect(&relay.url, n.private, s.to_bytes(), cfg).await.unwrap()
}

fn webp(n: usize, fill: u8) -> Vec<u8> {
    let mut p = [b"RIFF".as_slice(), &[0, 0, 0, 0], b"WEBPVP8 "].concat();
    p.resize(n, fill);
    p
}

/// O que a app de quem tem medalha envia.
fn submission(identity: &StaticSecret, photo: Option<&[u8]>) -> String {
    let subject = PublicKey::from(identity).to_bytes();
    let cert = badge::issue(&ISSUER, &subject, badge::BADGE_TESTER, 1_700_000_000_000);
    b64(&community::seal_submission_with(&report::operator_report_public(&SEED), identity, &cert, 5, photo).unwrap())
}

#[tokio::test]
async fn envio_guardado_selado_e_lido_so_pelo_operador() {
    let relay = TestRelay::start(cfg()).await;
    let alice = SigningKeyPair::generate();
    let mut a = client(&relay, &alice).await;

    // Sem conta registada: recusado.
    let me = StaticSecret::from([7u8; 32]);
    let r = a.request(ClientMessage::CommunitySubmit { sealed: submission(&me, None) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == NOT_REGISTERED), "{r:?}");
    a.register_bundle(valid_bundle(&alice, 11)).await.unwrap();

    let photo = webp(9_973, 3);
    let r = a.request(ClientMessage::CommunitySubmit { sealed: submission(&me, Some(&photo)) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    // Lixo, base64 invalido ou grande demais: recusado.
    for bad in ["".to_string(), "!!".to_string(), b64(&[1u8; 10]), b64(&vec![1u8; community::MAX_SUBMISSION + 1])] {
        let r = a.request(ClientMessage::CommunitySubmit { sealed: bad }).await.unwrap();
        assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
    }

    // Quem nao e operador nao lista nem publica.
    for m in [
        ClientMessage::AdminListCommunity { after: 0 },
        ClientMessage::AdminCommunityPublish { version: 1, index: b64(&[1]) },
        ClientMessage::AdminCommunityKeep { hashes: vec![] },
    ] {
        let r = a.request(m).await.unwrap();
        assert!(matches!(r, ServerMessage::Error { ref reason } if reason == NOT_OPERATOR), "{r:?}");
    }

    // O operador lista, abre com a semente (a foto e a da Alice) e apaga.
    let mut op = client(&relay, &operator()).await;
    let items = match op.request(ClientMessage::AdminListCommunity { after: 0 }).await.unwrap() {
        ServerMessage::CommunitySubmissions { items, more } => {
            assert!(!more);
            items
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(items.len(), 1);
    let got = community::open_submission(&SEED, &badge::issuer_public(&ISSUER), &unb64(&items[0].sealed)).unwrap();
    assert_eq!((got.cert.badge, got.photo), (badge::BADGE_TESTER, Some(photo)));
    let r = op.request(ClientMessage::AdminDeleteCommunity { id: items[0].id }).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack));
    match op.request(ClientMessage::AdminListCommunity { after: 0 }).await.unwrap() {
        ServerMessage::CommunitySubmissions { items, .. } => assert!(items.is_empty()),
        other => panic!("{other:?}"),
    }

    // Limite diario por conta (ja foi 1).
    for _ in 1..qt_server::store::MAX_COMMUNITY_SUBMITS_PER_DAY {
        let r = a.request(ClientMessage::CommunitySubmit { sealed: submission(&me, None) }).await.unwrap();
        assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    }
    let r = a.request(ClientMessage::CommunitySubmit { sealed: submission(&me, None) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == COMMUNITY_LIMIT), "{r:?}");
}

#[tokio::test]
async fn lista_publicada_lida_por_qualquer_conta_e_so_as_fotos_pedidas() {
    let relay = TestRelay::start(cfg()).await;
    let bob = SigningKeyPair::generate();
    let mut b = client(&relay, &bob).await;

    // Ainda sem lista.
    assert!(matches!(b.request(ClientMessage::CommunityVersion).await.unwrap(), ServerMessage::CommunityVersion { version: 0 }));
    assert!(matches!(b.request(ClientMessage::CommunityIndex).await.unwrap(), ServerMessage::CommunityNone));

    // O operador faz a lista com tres fotos.
    let photos: Vec<([u8; 32], Vec<u8>)> = (0..3).map(|i| community::seal_photo(&webp(9_973, i)).unwrap()).collect();
    let ix = community::Index {
        version: 17,
        created_ms: 1,
        members: photos
            .iter()
            .enumerate()
            .map(|(_, (h, _))| community::Member { badge: 1, photo_hash: Some(*h) })
            .collect(),
    };
    let index = community::build_index(&ISSUER, &ix).unwrap();
    let hashes: Vec<String> = photos.iter().map(|(h, _)| hex(h)).collect();
    let mut op = client(&relay, &operator()).await;
    match op.request(ClientMessage::AdminCommunityKeep { hashes: hashes.clone() }).await.unwrap() {
        ServerMessage::CommunityMissing { hashes: missing } => assert_eq!(missing, hashes),
        other => panic!("{other:?}"),
    }
    // Uma foto com o hash errado e recusada.
    let r = op.request(ClientMessage::AdminCommunityPhoto { hash: hashes[1].clone(), data: b64(&photos[0].1) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
    for (h, blob) in &photos {
        let r = op.request(ClientMessage::AdminCommunityPhoto { hash: hex(h), data: b64(blob) }).await.unwrap();
        assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    }
    match op.request(ClientMessage::AdminCommunityKeep { hashes: hashes.clone() }).await.unwrap() {
        ServerMessage::CommunityMissing { hashes: missing } => assert!(missing.is_empty()),
        other => panic!("{other:?}"),
    }
    let r = op.request(ClientMessage::AdminCommunityPublish { version: 17, index: b64(&index) }).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack), "{r:?}");

    // Qualquer conta (mesmo sem bundle) le a versao, o indice e as fotos.
    assert!(matches!(b.request(ClientMessage::CommunityVersion).await.unwrap(), ServerMessage::CommunityVersion { version: 17 }));
    let got = match b.request(ClientMessage::CommunityIndex).await.unwrap() {
        ServerMessage::CommunityIndex { version, data } => {
            assert_eq!(version, 17);
            community::open_index_with(&badge::issuer_public(&ISSUER), &community::COMMUNITY_KEY, &unb64(&data)).unwrap()
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(got, ix);
    // Pede as tres: vem as que cabem numa pagina; pede o resto a seguir.
    let mut have = Vec::new();
    let mut want = hashes.clone();
    while !want.is_empty() {
        let ServerMessage::CommunityPhotos { photos: page } = b.request(ClientMessage::CommunityPhotos { hashes: want.clone() }).await.unwrap() else {
            panic!("resposta inesperada")
        };
        assert!(!page.is_empty());
        for p in page {
            let h: [u8; 32] = photos.iter().find(|(h, _)| hex(h) == p.hash).unwrap().0;
            have.push(community::open_photo(&h, &unb64(&p.data)).unwrap());
            want.retain(|w| w != &p.hash);
        }
    }
    assert_eq!(have.len(), 3);
    // Hash desconhecido: nao vem nada. Pedido mal formado: recusado.
    let r = b.request(ClientMessage::CommunityPhotos { hashes: vec!["ab".repeat(32)] }).await.unwrap();
    assert!(matches!(r, ServerMessage::CommunityPhotos { ref photos } if photos.is_empty()), "{r:?}");
    let r = b.request(ClientMessage::CommunityPhotos { hashes: vec!["x".into()] }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
    let r = b.request(ClientMessage::CommunityPhotos { hashes: vec!["ab".repeat(32); MAX_COMMUNITY_PHOTOS_PER_REQUEST + 1] }).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
}
