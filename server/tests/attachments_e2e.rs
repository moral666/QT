//! Imagens/GIFs de ponta a ponta, pelo mesmo caminho que as apps:
//! WebSocket + NoiseSession + pedidos construidos pelo QtClient, relay real com
//! anexos no disco e Redis real. Inclui quedas de ligacao a meio do envio e da
//! descarga, ficheiro adulterado pelo relay e ficheiro expirado.

mod common;
use common::*;

use futures_util::{SinkExt, StreamExt};
use qt_ffi::*;
use qt_ffi::attachments::{AttachmentRef, AttachmentState};
use qt_ffi::history::{MessageStatus, ReceiveResult};
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn tmp(n: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_att_{}_{}_{}.sqlite", n, std::process::id(), id);
    let _ = std::fs::remove_file(&p);
    let _ = std::fs::remove_dir_all(format!("{p}.anexos"));
    p
}

struct User {
    path: String,
    qt: Arc<QtClient>,
    id: PublicIdentity,
}

fn user(name: &str, k: u8) -> User {
    let path = tmp(name);
    let qt = QtClient::open(path.clone(), vec![k; 32]).unwrap();
    let id = qt.create_identity().unwrap();
    User { path, qt, id }
}

struct Link {
    ws: Ws,
    noise: Arc<NoiseSession>,
}

async fn connect(qt: &Arc<QtClient>, url: &str) -> Link {
    let noise = qt.relay_noise_session().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    while !noise.is_finished() {
        ws.send(Message::Binary(noise.write_step().unwrap())).await.unwrap();
        if !noise.is_finished() {
            let Message::Binary(b) = ws.next().await.unwrap().unwrap() else { panic!() };
            noise.read_step(b).unwrap();
        }
    }
    let mut l = Link { ws, noise };
    let auth = qt.relay_auth_message(l.noise.handshake_hash().unwrap()).unwrap();
    assert!(matches!(l.rpc(auth).await, RelayResponse::Ack));
    l
}

impl Link {
    async fn rpc(&mut self, req: Vec<u8>) -> RelayResponse {
        self.many(vec![req]).await.pop().unwrap()
    }

    /// Varios pedidos seguidos e so depois as respostas, pela mesma ordem (como
    /// as apps fazem para nao esperar uma volta inteira por cada bloco).
    async fn many(&mut self, reqs: Vec<Vec<u8>>) -> Vec<RelayResponse> {
        for r in &reqs {
            assert!(r.len() < 65_519, "pedido maior que uma mensagem Noise: {}", r.len());
            let ct = self.noise.encrypt(r.clone()).unwrap();
            self.ws.send(Message::Binary(ct)).await.unwrap();
        }
        let mut out = Vec::new();
        while out.len() < reqs.len() {
            match self.ws.next().await.unwrap().unwrap() {
                Message::Binary(b) => out.push(relay_parse_response(self.noise.decrypt(b).unwrap()).unwrap()),
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("{other:?}"),
            }
        }
        out
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn random_file(name: &str, len: usize) -> (String, Vec<u8>) {
    use std::io::Read;
    let mut data = vec![0u8; len];
    std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut data).unwrap();
    let p = format!("/tmp/qt_att_src_{}_{}_{}", name, std::process::id(), len);
    std::fs::write(&p, &data).unwrap();
    (p, data)
}

const WINDOW: usize = 8;

/// Envia o anexo como a app: reserva, retoma pelo que o relay ja tem, blocos em janela.
/// `stop_after` simula a ligacao a cair depois de N blocos.
async fn upload(qt: &Arc<QtClient>, l: &mut Link, r: &AttachmentRef, stop_after: Option<usize>) -> bool {
    let (cid, mid) = (r.contact_id.clone(), r.message_id.clone());
    let mut start = 0u64;
    if qt.attachment_blob_id(cid.clone(), mid.clone()).unwrap().is_none() {
        let RelayResponse::BlobCreated { blob_id, token } = l.rpc(qt.blob_create_request(cid.clone(), mid.clone()).unwrap()).await else {
            panic!("BlobCreate")
        };
        qt.attachment_set_blob(cid.clone(), mid.clone(), blob_id, token).unwrap();
    } else {
        let RelayResponse::BlobStatus { received, .. } = l.rpc(qt.blob_info_request(cid.clone(), mid.clone()).unwrap()).await else {
            panic!("BlobInfo")
        };
        start = received - received % BLOB_CHUNK_BYTES;
    }
    let total = qt.messages_page(cid.clone(), None, 50).unwrap().into_iter().find(|m| m.message_id == mid).unwrap().attachment.unwrap().transfer_size;
    let offsets: Vec<u64> = (start..total).step_by(BLOB_CHUNK_BYTES as usize).collect();
    let mut sent = 0;
    for window in offsets.chunks(WINDOW) {
        let reqs = window.iter().map(|o| qt.blob_put_request(cid.clone(), mid.clone(), *o).unwrap()).collect();
        for resp in l.many(reqs).await {
            assert!(matches!(resp, RelayResponse::BlobStatus { .. }), "{}", describe(&resp));
        }
        sent += window.len();
        if stop_after.is_some_and(|n| sent >= n) {
            return false;
        }
    }
    true
}

const BLOB_CHUNK_BYTES: u64 = 40 * 1024;

fn describe(r: &RelayResponse) -> String {
    match r {
        RelayResponse::Error { reason } => reason.clone(),
        RelayResponse::Ack => "Ack".into(),
        RelayResponse::BlobNotFound => "BlobNotFound".into(),
        _ => "outra".into(),
    }
}

/// Descarrega como a app, a partir do que ja esta no telemovel.
async fn download(qt: &Arc<QtClient>, l: &mut Link, r: &AttachmentRef, stop_after: Option<usize>) -> Result<bool, ()> {
    let (cid, mid) = (r.contact_id.clone(), r.message_id.clone());
    let mut have = qt.attachment_downloaded_bytes(cid.clone(), mid.clone()).unwrap();
    let mut n = 0;
    loop {
        match l.rpc(qt.blob_get_request(cid.clone(), mid.clone(), have).unwrap()).await {
            RelayResponse::BlobData { offset, size, data } => {
                assert_eq!(offset, have);
                have = qt.attachment_write_chunk(cid.clone(), mid.clone(), offset, data).unwrap();
                if have == size {
                    return Ok(true);
                }
            }
            RelayResponse::BlobNotFound => return Err(()),
            other => panic!("{}", describe(&other)),
        }
        n += 1;
        if stop_after.is_some_and(|s| n >= s) {
            return Ok(false);
        }
    }
}

async fn setup(relay: &TestRelay) -> (User, User, Link, Link, String) {
    let (alice, bob) = (user("alice", 0xA1), user("bob", 0xB2));
    let mut la = connect(&alice.qt, &relay.url).await;
    let mut lb = connect(&bob.qt, &relay.url).await;
    assert!(matches!(lb.rpc(bob.qt.relay_request_register_bundle(bob.qt.publishable_bundle().unwrap()).unwrap()).await, RelayResponse::Ack));
    let RelayResponse::Bundle { bytes } = la.rpc(alice.qt.relay_request_fetch_bundle(bob.id.relay_id.clone()).unwrap()).await else { panic!() };
    let cid = alice.qt.start_session(bob.id.public_id.clone(), peer_bundle_from_relay_bytes(bytes).unwrap()).unwrap();
    (alice, bob, la, lb, cid)
}

/// Alice envia o anexo `mid` ja carregado; Bob recebe a mensagem. Devolve a referencia do lado de Bob.
async fn deliver(alice: &User, bob: &User, la: &mut Link, lb: &mut Link, cid: &str, mid: &str) -> AttachmentRef {
    let item = alice.qt.attachment_uploaded(cid.to_string(), mid.to_string()).unwrap();
    assert!(matches!(la.rpc(alice.qt.relay_request_send(bob.id.relay_id.clone(), item.blob).unwrap()).await, RelayResponse::Ack));
    alice.qt.outbox_sent(item.outbox_id).unwrap();
    let RelayResponse::Messages { messages } = lb.rpc(bob.qt.relay_request_fetch().unwrap()).await else { panic!() };
    assert_eq!(messages.len(), 1);
    let ReceiveResult::NewMessage { message } = bob.qt.receive_sealed(messages[0].ciphertext.clone()).unwrap() else { panic!() };
    assert!(matches!(lb.rpc(bob.qt.relay_request_ack(vec![messages[0].id]).unwrap()).await, RelayResponse::Ack));
    let att = message.attachment.clone().unwrap();
    assert_eq!(att.state, AttachmentState::Downloading);
    AttachmentRef { contact_id: message.contact_id, message_id: message.message_id }
}

#[tokio::test]
async fn imagem_de_ponta_a_ponta_com_quedas_de_rede() {
    let dir = std::env::temp_dir().join(format!("qt-att-relay-{}", std::process::id()));
    let relay = TestRelay::start_with_blobs(fast_cfg(), qt_server::BlobConfig::new(&dir)).await;
    let (alice, bob, mut la, mut lb, cid) = setup(&relay).await;

    // ~1,3 MB: varios blocos e um ultimo incompleto.
    let (src, original) = random_file("foto", 1_300_000);
    let m = alice.qt.attachment_create(cid.clone(), src.clone(), "image/jpeg".into(), 1600, 1200).unwrap();
    let att = m.attachment.clone().unwrap();
    assert_eq!((att.state, att.size, att.width, att.height), (AttachmentState::Uploading, 1_300_000, 1600, 1200));
    assert_eq!(m.text, "📷 Foto");
    assert_eq!(m.status, MessageStatus::Pending);
    // Quem envia ve logo a propria imagem.
    assert_eq!(alice.qt.attachment_read(cid.clone(), m.message_id.clone()).unwrap(), original);
    // O ficheiro no disco esta cifrado (nao contem os bytes originais).
    let on_disk: Vec<_> = std::fs::read_dir(format!("{}.anexos", alice.path)).unwrap().collect();
    assert_eq!(on_disk.len(), 1);
    let ct = std::fs::read(on_disk[0].as_ref().unwrap().path()).unwrap();
    assert!(!ct.windows(64).any(|w| w == &original[1000..1064]));
    assert!(ct.len() > original.len() && ct.len() as f64 <= original.len() as f64 * 1.13);

    let pending = alice.qt.attachments_pending_upload().unwrap();
    assert_eq!(pending.len(), 1);
    // Envio interrompido a meio: a ligacao cai e volta.
    assert!(!upload(&alice.qt, &mut la, &pending[0], Some(16)).await);
    drop(la);
    let mut la = connect(&alice.qt, &relay.url).await;
    assert!(upload(&alice.qt, &mut la, &pending[0], None).await, "retoma onde ficou");

    let bref = deliver(&alice, &bob, &mut la, &mut lb, &cid, &m.message_id).await;
    assert!(alice.qt.attachments_pending_upload().unwrap().is_empty());
    assert_eq!(alice.qt.messages_page(cid.clone(), None, 10).unwrap()[0].status, MessageStatus::Sent);
    assert_eq!(bob.qt.attachments_pending_download().unwrap().len(), 1);
    assert_eq!(bob.qt.conversations().unwrap()[0].last_message.text, "📷 Foto");
    // Ainda nao descarregada: nao se le.
    assert!(bob.qt.attachment_read(bref.contact_id.clone(), bref.message_id.clone()).is_err());

    // Descarga interrompida e retomada noutra ligacao.
    assert_eq!(download(&bob.qt, &mut lb, &bref, Some(10)).await, Ok(false));
    drop(lb);
    let mut lb = connect(&bob.qt, &relay.url).await;
    assert_eq!(download(&bob.qt, &mut lb, &bref, None).await, Ok(true));
    let done = bob.qt.attachment_download_finished(bref.contact_id.clone(), bref.message_id.clone()).unwrap();
    assert_eq!(done.attachment.unwrap().state, AttachmentState::Ready);
    assert_eq!(bob.qt.attachment_read(bref.contact_id.clone(), bref.message_id.clone()).unwrap(), original);
    assert!(bob.qt.attachments_pending_download().unwrap().is_empty());

    // Bob pede ao relay para apagar: o ficheiro desaparece do disco do relay.
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    assert!(matches!(lb.rpc(bob.qt.blob_delete_request(bref.contact_id.clone(), bref.message_id.clone()).unwrap()).await, RelayResponse::Ack));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);

    // Eliminar a conversa apaga tambem os ficheiros no telemovel.
    bob.qt.delete_conversation(bref.contact_id.clone()).unwrap();
    assert_eq!(std::fs::read_dir(format!("{}.anexos", bob.path)).unwrap().count(), 0);
    // Apagar todos os dados apaga a pasta inteira.
    alice.qt.delete_identity().unwrap();
    assert!(!std::path::Path::new(&format!("{}.anexos", alice.path)).exists());
    let _ = std::fs::remove_file(src);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn relay_que_troca_bytes_e_ficheiro_expirado() {
    let dir = std::env::temp_dir().join(format!("qt-att-relay2-{}", std::process::id()));
    let relay = TestRelay::start_with_blobs(fast_cfg(), qt_server::BlobConfig::new(&dir)).await;
    let (alice, bob, mut la, mut lb, cid) = setup(&relay).await;

    // GIF pequeno (1 bloco).
    let (src, _) = random_file("gif", 30_000);
    let m = alice.qt.attachment_create(cid.clone(), src.clone(), "image/gif".into(), 320, 240).unwrap();
    assert_eq!(m.text, "GIF");
    let r = alice.qt.attachments_pending_upload().unwrap().remove(0);
    assert!(upload(&alice.qt, &mut la, &r, None).await);
    let blob_id = alice.qt.attachment_blob_id(cid.clone(), m.message_id.clone()).unwrap().unwrap();
    let bref = deliver(&alice, &bob, &mut la, &mut lb, &cid, &m.message_id).await;

    // Um relay malicioso troca um byte do ficheiro guardado.
    let path = dir.join(&blob_id);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[100] ^= 0xFF;
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(download(&bob.qt, &mut lb, &bref, None).await, Ok(true));
    let done = bob.qt.attachment_download_finished(bref.contact_id.clone(), bref.message_id.clone()).unwrap();
    assert_eq!(done.attachment.unwrap().state, AttachmentState::Failed, "a alteracao e detetada");
    assert!(bob.qt.attachment_read(bref.contact_id.clone(), bref.message_id.clone()).is_err());
    assert!(bob.qt.attachments_pending_download().unwrap().is_empty());

    // Segundo anexo: o relay ja o apagou (24 h) antes de Bob o descarregar.
    let m2 = alice.qt.attachment_create(cid.clone(), src.clone(), "image/png".into(), 10, 10).unwrap();
    let r2 = alice.qt.attachments_pending_upload().unwrap().remove(0);
    assert!(upload(&alice.qt, &mut la, &r2, None).await);
    let blob2 = alice.qt.attachment_blob_id(cid.clone(), m2.message_id.clone()).unwrap().unwrap();
    let bref2 = deliver(&alice, &bob, &mut la, &mut lb, &cid, &m2.message_id).await;
    std::fs::remove_file(dir.join(&blob2)).unwrap();
    assert_eq!(download(&bob.qt, &mut lb, &bref2, None).await, Err(()));
    let gone = bob.qt.attachment_failed(bref2.contact_id.clone(), bref2.message_id.clone(), true).unwrap();
    assert_eq!(gone.attachment.unwrap().state, AttachmentState::Expired);

    // Entradas invalidas sao recusadas sem panic.
    assert!(alice.qt.attachment_create(cid.clone(), src.clone(), "application/pdf".into(), 1, 1).is_err());
    assert!(alice.qt.attachment_create(cid.clone(), "/nao/existe".into(), "image/png".into(), 1, 1).is_err());
    assert!(alice.qt.attachment_create("zz".into(), src.clone(), "image/png".into(), 1, 1).is_err());
    assert!(alice.qt.blob_put_request(cid.clone(), m2.message_id.clone(), 7).is_err(), "offset fora dos blocos");
    assert!(alice.qt.attachment_set_blob(cid.clone(), m2.message_id.clone(), "../x".into(), "y".into()).is_err());
    assert!(bob.qt.attachment_write_chunk(bref.contact_id.clone(), hex(&[0u8; 16]), 0, vec![1]).is_err());
    let _ = std::fs::remove_file(src);
    let _ = std::fs::remove_dir_all(dir);
}
