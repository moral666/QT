//! Ficheiros (PDF, documentos...) ate 25 MiB, a dois e em grupos, e fotos/voz nos
//! grupos. O "relay" e simulado: os blocos cifrados passam de quem envia para quem
//! recebe tal como o relay os guardaria (BlobPut -> BlobGet), e os envelopes da
//! outbox sao entregues ao destinatario.

use base64::Engine;
use qt_ffi::attachments::*;
use qt_ffi::history::*;
use qt_ffi::*;
use std::sync::Arc;

fn tmp(n: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_fich_{}_{}_{}.sqlite", n, std::process::id(), id);
    let _ = std::fs::remove_file(&p);
    p
}

struct Peer {
    c: Arc<QtClient>,
    id: PublicIdentity,
    db: String,
}

fn peer(name: &str, key: u8) -> Peer {
    let db = tmp(name);
    let c = QtClient::open(db.clone(), vec![key; 32]).unwrap();
    let id = c.create_identity().unwrap();
    Peer { c, id, db }
}

impl Peer {
    fn cid(&self) -> String {
        self.id.identity_public.iter().map(|b| format!("{b:02x}")).collect()
    }
    fn pid(&self) -> String {
        self.id.public_id.clone()
    }
    fn session_with(&self, other: &Peer) {
        if !self.c.has_session(other.cid()).unwrap() {
            let bundle = peer_bundle_from_relay_bytes(other.c.publishable_bundle().unwrap()).unwrap();
            self.c.start_session(other.pid(), bundle).unwrap();
        }
    }
}

/// Entrega a outbox de `from` aos destinatarios que estao em `all`.
fn route(from: &Peer, all: &[&Peer]) {
    for item in from.c.outbox().unwrap() {
        if let Some(to) = all.iter().find(|p| p.cid() == item.contact_id) {
            to.c.receive_sealed(item.blob.clone()).unwrap();
        }
        from.c.outbox_sent(item.outbox_id).unwrap();
    }
}

fn source(name: &str, bytes: &[u8]) -> String {
    let f = format!("/tmp/qt_fich_src_{}_{}", std::process::id(), name);
    std::fs::write(&f, bytes).unwrap();
    f
}

/// "Envia ao relay": guarda os blocos cifrados que a app mandaria em BlobPut.
fn upload(p: &Peer, conv: &str, mid: &str) -> Vec<u8> {
    p.c.attachment_set_blob(conv.into(), mid.into(), "ab".repeat(16), "cd".repeat(16)).unwrap();
    let total = p.c.message(conv.into(), mid.into()).unwrap().unwrap().attachment.unwrap().transfer_size;
    let mut relay = Vec::new();
    let mut off = 0u64;
    while off < total {
        let req: serde_json::Value = serde_json::from_slice(&p.c.blob_put_request(conv.into(), mid.into(), off).unwrap()).unwrap();
        let data = base64::engine::general_purpose::STANDARD.decode(req["BlobPut"]["data"].as_str().unwrap()).unwrap();
        off += data.len() as u64;
        relay.extend_from_slice(&data);
    }
    relay
}

/// "Descarrega do relay" para quem recebeu e verifica.
fn download(p: &Peer, conv: &str, mid: &str, relay: &[u8]) -> ChatMessage {
    for (i, chunk) in relay.chunks(BLOB_CHUNK).enumerate() {
        p.c.attachment_write_chunk(conv.into(), mid.into(), (i * BLOB_CHUNK) as u64, chunk.to_vec()).unwrap();
    }
    p.c.attachment_download_finished(conv.into(), mid.into()).unwrap()
}

#[test]
fn ficheiro_a_dois_chega_com_nome_e_tipo() {
    let (a, b) = (peer("alice", 0x41), peer("bob", 0x42));
    a.session_with(&b);
    let content: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let f = source("relatorio.pdf", &content);

    let m = a.c.attachment_create_file(b.cid(), f.clone(), "application/pdf".into(), "Relatório final.pdf".into()).unwrap();
    assert_eq!(m.text, "📄 Relatório final.pdf");
    let att = m.attachment.clone().unwrap();
    assert_eq!(att.file_name.as_deref(), Some("Relatório final.pdf"));
    assert_eq!((att.width, att.height), (0, 0));
    assert_eq!(a.c.attachments_pending_upload().unwrap().len(), 1);

    let relay = upload(&a, &b.cid(), &m.message_id);
    a.c.attachment_uploaded(b.cid(), m.message_id.clone()).unwrap();
    route(&a, &[&b]);

    let pend = b.c.attachments_pending_download().unwrap();
    assert_eq!(pend.len(), 1);
    assert_eq!(pend[0].message_id, m.message_id);
    let got = download(&b, &a.cid(), &m.message_id, &relay);
    let gatt = got.attachment.unwrap();
    assert_eq!(gatt.state, AttachmentState::Ready);
    assert_eq!((gatt.mime.as_str(), gatt.file_name.as_deref()), ("application/pdf", Some("Relatório final.pdf")));
    assert_eq!(got.text, "📄 Relatório final.pdf");
    assert_eq!(b.c.attachment_read(a.cid(), m.message_id.clone()).unwrap(), content);
    // A dois, quem recebe pode pedir ao relay para apagar.
    assert!(b.c.blob_delete_request(a.cid(), m.message_id).is_ok());
    let _ = std::fs::remove_file(f);
}

#[test]
fn ficheiro_com_nome_ou_tipo_invalido_e_recusado() {
    let (a, b) = (peer("alice", 0x51), peer("bob", 0x52));
    a.session_with(&b);
    let f = source("x.bin", &[1, 2, 3]);
    for (mime, name) in [
        ("application/pdf", ""),
        ("application/pdf", "  "),
        ("application/pdf", "../segredo.txt"),
        ("application/pdf", "pasta\\nome.txt"),
        ("application/pdf", "linha\nnova.txt"),
        ("application/pdf", ".."),
        ("PDF", "a.pdf"),
        ("application/PDF", "a.pdf"),
        ("application/pdf; charset=x", "a.pdf"),
        ("/pdf", "a.pdf"),
    ] {
        assert!(a.c.attachment_create_file(b.cid(), f.clone(), mime.into(), name.into()).is_err(), "{mime:?} {name:?}");
    }
    let long = "a".repeat(MAX_FILE_NAME_BYTES + 1);
    assert!(a.c.attachment_create_file(b.cid(), f.clone(), "text/plain".into(), long).is_err());
    // Tipos de imagem ou outros, com nome valido: aceites como ficheiro.
    for mime in ["image/png", "application/vnd.openxmlformats-officedocument.wordprocessingml.document", "application/octet-stream"] {
        assert!(a.c.attachment_create_file(b.cid(), f.clone(), mime.into(), "doc".into()).is_ok(), "{mime}");
    }
    // Um tipo qualquer sem nome continua recusado pelo caminho das imagens.
    assert!(a.c.attachment_create(b.cid(), f.clone(), "application/pdf".into(), 0, 0).is_err());
    // Maior que 25 MiB: recusado.
    let big = source("grande.bin", &vec![0u8; (MAX_ATTACHMENT_BYTES + 1) as usize]);
    assert!(a.c.attachment_create_file(b.cid(), big.clone(), "application/zip".into(), "grande.zip".into()).is_err());
    let _ = std::fs::remove_file(f);
    let _ = std::fs::remove_file(big);
}

/// Alice cria o grupo com Bob e Carol; todos com sessao entre si.
fn group3() -> (Peer, Peer, Peer, String, String) {
    let (a, b, c) = (peer("alice", 0x61), peer("bob", 0x62), peer("carol", 0x63));
    a.session_with(&b);
    a.session_with(&c);
    let g = a.c.group_create("Equipa".into(), vec![b.pid(), c.pid()]).unwrap();
    route(&a, &[&b, &c]);
    b.session_with(&c);
    c.session_with(&b);
    (a, b, c, g.group_id, g.conversation_id)
}

#[test]
fn foto_voz_e_ficheiro_num_grupo_chegam_a_todos() {
    let (a, b, c, gid, conv) = group3();
    let photo = source("foto.jpg", &vec![0xEE; 120_000]);
    let voice = source("voz.m4a", &vec![0x11; 30_000]);
    let doc = source("ata.pdf", &vec![0x22; 90_000]);

    let sent = [
        a.c.attachment_create(conv.clone(), photo.clone(), "image/jpeg".into(), 800, 600).unwrap(),
        a.c.attachment_create(conv.clone(), voice.clone(), "audio/mp4".into(), 4_000, 0).unwrap(),
        a.c.attachment_create_file(conv.clone(), doc.clone(), "application/pdf".into(), "Ata.pdf".into()).unwrap(),
    ];
    let mut relays = Vec::new();
    for m in &sent {
        assert_eq!(m.contact_id, conv);
        assert_eq!(m.status, MessageStatus::Pending);
        relays.push(upload(&a, &conv, &m.message_id));
        // O caminho a dois nao serve para grupos.
        assert!(a.c.attachment_uploaded(conv.clone(), m.message_id.clone()).is_err());
        assert!(a.c.group_attachment_uploaded(gid.clone(), m.message_id.clone()).unwrap().is_empty());
    }
    // Um envelope por membro e por anexo; "Enviada" quando o relay confirma todos.
    assert_eq!(a.c.outbox().unwrap().len(), 6);
    route(&a, &[&b, &c]);
    for m in &sent {
        assert_eq!(a.c.message(conv.clone(), m.message_id.clone()).unwrap().unwrap().status, MessageStatus::Sent);
    }

    for p in [&b, &c] {
        let pend = p.c.attachments_pending_download().unwrap();
        assert_eq!(pend.len(), 3);
        assert!(pend.iter().all(|r| r.contact_id == conv));
        for (m, relay) in sent.iter().zip(&relays) {
            let got = download(p, &conv, &m.message_id, relay);
            assert_eq!(got.attachment.unwrap().state, AttachmentState::Ready);
            assert_eq!(got.sender, a.cid(), "guarda quem enviou");
            // Nos grupos ninguem apaga do relay: os outros ainda podem estar a descarregar.
            assert!(p.c.blob_delete_request(conv.clone(), m.message_id.clone()).is_err());
        }
        let page = p.c.group_messages_page(gid.clone(), None, 10).unwrap();
        let labels: Vec<_> = page.iter().rev().map(|m| m.text.clone()).collect();
        assert_eq!(labels, vec!["📷 Foto", "🎤 Mensagem de voz", "📄 Ata.pdf"]);
        assert_eq!(p.c.attachment_read(conv.clone(), sent[2].message_id.clone()).unwrap(), vec![0x22; 90_000]);
    }

    // Apagar o grupo apaga tambem os ficheiros cifrados no telemovel.
    let n_before = std::fs::read_dir(format!("{}.anexos", tmp_dir_of(&b))).map(|d| d.count()).unwrap_or(0);
    assert!(n_before >= 3);
    b.c.group_delete(gid.clone()).unwrap();
    let n_after = std::fs::read_dir(format!("{}.anexos", tmp_dir_of(&b))).map(|d| d.count()).unwrap_or(0);
    assert_eq!(n_after, 0);

    for f in [photo, voice, doc] {
        let _ = std::fs::remove_file(f);
    }
}

/// Caminho da base de dados de um cliente (a pasta de anexos e `<base>.anexos`).
fn tmp_dir_of(p: &Peer) -> String {
    p.db.clone()
}

#[test]
fn quem_ja_nao_esta_no_grupo_nao_envia_anexos() {
    let (a, b, c, gid, conv) = group3();
    b.c.group_leave(gid.clone()).unwrap();
    route(&b, &[&a, &c]);
    let f = source("x.pdf", &[9; 1000]);
    assert!(b.c.attachment_create_file(conv.clone(), f.clone(), "application/pdf".into(), "x.pdf".into()).is_err());
    assert!(b.c.attachment_create(conv.clone(), f.clone(), "image/png".into(), 10, 10).is_err());
    // Conversa de grupo que nao existe: recusada.
    assert!(a.c.attachment_create(format!("g:{}", "00".repeat(16)), f.clone(), "image/png".into(), 10, 10).is_err());
    let _ = std::fs::remove_file(f);
}
