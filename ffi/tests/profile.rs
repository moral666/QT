//! Foto de perfil (tipo 12): so contactos, cifrada, de 72 em 72 h.

use qt_ffi::history::*;
use qt_ffi::*;
use std::sync::Arc;

fn tmp(n: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_prof_{}_{}_{}.sqlite", n, std::process::id(), id);
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
    (a, b)
}

fn webp(n: usize, fill: u8) -> Vec<u8> {
    let mut p = [b"RIFF".as_slice(), &[0, 0, 0, 0], b"WEBPVP8 "].concat();
    p.resize(n, fill);
    p
}

#[test]
fn foto_chega_ao_contacto_e_so_muda_de_72_em_72_h() {
    let (a, b) = pair();
    assert_eq!(a.c.profile_photo_next_change_ms().unwrap(), 0);
    let foto = webp(9_000, 7);
    assert_eq!(a.c.profile_photo_set(foto.clone(), vec![b.cid()]).unwrap(), 1);
    let r = relay(&a, &b);
    assert!(matches!(&r[0], ReceiveResult::Updated { contact_id } if *contact_id == a.cid()));
    assert_eq!(b.c.profile_photo_of(a.cid()).unwrap(), Some(foto.clone()));
    assert_eq!(a.c.profile_photo_mine().unwrap(), Some(foto));

    // Antes das 72 h nao da para trocar.
    assert!(a.c.profile_photo_next_change_ms().unwrap() > 71 * 3_600_000);
    assert!(matches!(a.c.profile_photo_set(webp(500, 1), vec![b.cid()]), Err(QtError::InvalidInput { field }) if field == "photo_wait"));

    // Sincronizar outra vez nao repete (o Bob ja tem esta versao).
    assert_eq!(a.c.profile_photo_sync(vec![b.cid()]).unwrap(), 0);

    // Tirar a foto pode-se sempre e o Bob deixa de a ver.
    assert_eq!(a.c.profile_photo_remove(vec![b.cid()]).unwrap(), 1);
    relay(&a, &b);
    assert_eq!(b.c.profile_photo_of(a.cid()).unwrap(), None);
    assert!(a.c.profile_photo_next_change_ms().unwrap() > 0);
}

#[test]
fn formato_e_tamanho_errados_sao_recusados() {
    let (a, b) = pair();
    assert!(a.c.profile_photo_set(b"GIF89a....".to_vec(), vec![b.cid()]).is_err());
    assert!(a.c.profile_photo_set(webp(10_001, 0), vec![b.cid()]).is_err());
    assert!(a.c.outbox().unwrap().is_empty());
}

#[test]
fn contacto_novo_recebe_na_sincronizacao_e_quem_nao_e_contacto_nao() {
    let (a, b) = pair();
    // Sem foto e sem nada enviado: nada a dizer.
    assert_eq!(a.c.profile_photo_sync(vec![b.cid()]).unwrap(), 0);
    // A foto e escolhida sem o Bob na lista; o Bob recebe-a na sincronizacao seguinte.
    assert_eq!(a.c.profile_photo_set(webp(2_000, 3), vec![]).unwrap(), 0);
    assert_eq!(a.c.profile_photo_sync(vec![b.cid()]).unwrap(), 1);
    relay(&a, &b);
    assert!(b.c.profile_photo_of(a.cid()).unwrap().is_some());
    // Apagar o contacto apaga a foto dele.
    b.c.delete_contact(a.cid()).unwrap();
    assert_eq!(b.c.profile_photo_of(a.cid()).unwrap(), None);
}
