//! Anexos no relay: envio por blocos, retomar, permissoes, leitura, apagar,
//! limites e limpeza dos expirados. Precisa de um Redis em 127.0.0.1:6379.

mod common;
use common::*;

use base64::Engine;
use qt_core::primitives::SigningKeyPair;
use qt_server::blobs::{BLOB_CHUNK, MAX_BLOB_BYTES};
use qt_server::protocol::*;
use qt_server::{BlobConfig, Store};
use qt_transport::{generate_static_keypair, ws_transport};
use std::time::Duration;

type Ws = ws_transport::ClientWsStream;
type Noise = qt_transport::NoiseTransport;

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("qt-blobs-{tag}-{}-{}", std::process::id(), rand_u32()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn rand_u32() -> u32 {
    let mut b = [0u8; 4];
    getrandom(&mut b);
    u32::from_le_bytes(b)
}

fn getrandom(b: &mut [u8]) {
    use std::io::Read;
    std::fs::File::open("/dev/urandom").unwrap().read_exact(b).unwrap();
}

/// Liga e autentica uma conta registada (so contas registadas criam anexos).
async fn login(relay: &TestRelay, s: &SigningKeyPair) -> (Ws, Noise) {
    registar(relay, &user_id_from_signing_public(&s.verifying_key.to_bytes())).await;
    let k = generate_static_keypair().unwrap();
    let (mut ws, mut n) = ws_transport::client_connect(&relay.url, &k.private).await.unwrap();
    let mut m = AUTH_CONTEXT.to_vec();
    m.extend_from_slice(n.handshake_hash());
    let auth = ClientMessage::Authenticate {
        signing_public: s.verifying_key.to_bytes().to_vec(),
        signature: s.sign(&m).to_bytes().to_vec(),
    };
    assert!(matches!(req(&mut ws, &mut n, &auth).await, ServerMessage::Ack));
    (ws, n)
}

async fn req(ws: &mut Ws, n: &mut Noise, m: &ClientMessage) -> ServerMessage {
    let bytes = serialize_client_message(m);
    assert!(bytes.len() < 65_519, "pedido maior que uma mensagem Noise: {}", bytes.len());
    ws_transport::send_encrypted(ws, n, &bytes).await.unwrap();
    let b = ws_transport::receive_encrypted_timeout(ws, n, Duration::from_secs(5)).await.unwrap();
    deserialize_server_message(&b).unwrap()
}

fn cfg(dir: std::path::PathBuf) -> BlobConfig {
    let mut c = BlobConfig::new(dir);
    c.max_total_bytes = 64 * 1024 * 1024;
    c.max_user_bytes_per_day = 40 * 1024 * 1024;
    c.min_free_bytes = 0;
    c
}

#[tokio::test]
async fn envio_por_blocos_leitura_e_apagar() {
    let dir = tmpdir("fluxo");
    let relay = TestRelay::start_with_blobs(fast_cfg(), cfg(dir.clone())).await;
    let alice = SigningKeyPair::generate();
    let bob = SigningKeyPair::generate();
    let (mut wa, mut na) = login(&relay, &alice).await;
    let (mut wb, mut nb) = login(&relay, &bob).await;

    // ~2,5 blocos de bytes "cifrados" (para o relay sao so bytes).
    let mut file = vec![0u8; BLOB_CHUNK * 2 + 1234];
    getrandom(&mut file);
    let ServerMessage::BlobCreated { blob_id, token } =
        req(&mut wa, &mut na, &ClientMessage::BlobCreate { size: file.len() as u64 }).await
    else {
        panic!("BlobCreate falhou")
    };

    // O ficheiro nao pode ser lido antes de estar completo.
    let r = req(&mut wb, &mut nb, &ClientMessage::BlobGet { blob_id: blob_id.clone(), offset: 0 }).await;
    assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");

    let put = |offset: usize, token: &str| ClientMessage::BlobPut {
        blob_id: blob_id.clone(),
        token: token.to_string(),
        offset: offset as u64,
        data: b64(&file[offset..(offset + BLOB_CHUNK).min(file.len())]),
    };
    // Token errado: recusado.
    let r = req(&mut wb, &mut nb, &put(0, &"0".repeat(32))).await;
    assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
    // Primeiro bloco.
    let r = req(&mut wa, &mut na, &put(0, &token)).await;
    assert!(matches!(r, ServerMessage::BlobStatus { received, complete: false, .. } if received == BLOB_CHUNK as u64), "{r:?}");
    // Saltar um bloco: recusado. Repetir o primeiro: aceite sem mudar nada.
    let r = req(&mut wa, &mut na, &put(2 * BLOB_CHUNK, &token)).await;
    assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
    let r = req(&mut wa, &mut na, &put(0, &token)).await;
    assert!(matches!(r, ServerMessage::BlobStatus { received, .. } if received == BLOB_CHUNK as u64), "{r:?}");
    // Retomar: o relay diz quanto ja tem.
    let r = req(&mut wa, &mut na, &ClientMessage::BlobInfo { blob_id: blob_id.clone() }).await;
    assert!(matches!(r, ServerMessage::BlobStatus { received, complete: false, .. } if received == BLOB_CHUNK as u64), "{r:?}");
    req(&mut wa, &mut na, &put(BLOB_CHUNK, &token)).await;
    let r = req(&mut wa, &mut na, &put(2 * BLOB_CHUNK, &token)).await;
    assert!(matches!(r, ServerMessage::BlobStatus { complete: true, .. }), "{r:?}");

    // O destinatario le tudo, bloco a bloco, e o conteudo e igual.
    let mut got = Vec::new();
    while got.len() < file.len() {
        let r = req(&mut wb, &mut nb, &ClientMessage::BlobGet { blob_id: blob_id.clone(), offset: got.len() as u64 }).await;
        let ServerMessage::BlobData { offset, size, data } = r else { panic!("{r:?}") };
        assert_eq!((offset, size), (got.len() as u64, file.len() as u64));
        got.extend(base64::engine::general_purpose::STANDARD.decode(data).unwrap());
    }
    assert_eq!(got, file);
    assert!(dir.join(&blob_id).exists());

    // Apagar: o ficheiro desaparece do disco e deixa de existir para todos.
    assert!(matches!(req(&mut wb, &mut nb, &ClientMessage::BlobDelete { blob_id: blob_id.clone() }).await, ServerMessage::Ack));
    assert!(!dir.join(&blob_id).exists());
    let r = req(&mut wa, &mut na, &ClientMessage::BlobGet { blob_id: blob_id.clone(), offset: 0 }).await;
    assert!(matches!(r, ServerMessage::BlobNotFound), "{r:?}");
    // Apagar outra vez nao e erro.
    assert!(matches!(req(&mut wb, &mut nb, &ClientMessage::BlobDelete { blob_id }).await, ServerMessage::Ack));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn limites_de_tamanho_quota_e_ids_invalidos() {
    let dir = tmpdir("limites");
    let relay = TestRelay::start_with_blobs(fast_cfg(), cfg(dir.clone())).await;
    let a = SigningKeyPair::generate();
    let (mut ws, mut n) = login(&relay, &a).await;

    for size in [0, MAX_BLOB_BYTES + 1] {
        let r = req(&mut ws, &mut n, &ClientMessage::BlobCreate { size }).await;
        assert!(matches!(r, ServerMessage::Error { .. }), "{size}: {r:?}");
    }
    // Quota diaria de 40 MiB: 25 + 10 passa, mais 10 ja nao.
    for (size, ok) in [(25u64 << 20, true), (10 << 20, true), (10 << 20, false)] {
        let r = req(&mut ws, &mut n, &ClientMessage::BlobCreate { size }).await;
        assert_eq!(matches!(r, ServerMessage::BlobCreated { .. }), ok, "{size}: {r:?}");
    }
    // Ids que nao sao 32 hex (ex.: caminhos) sao recusados sem tocar no disco.
    for bad in ["../../etc/passwd", "", "ZZ"] {
        let r = req(&mut ws, &mut n, &ClientMessage::BlobGet { blob_id: bad.into(), offset: 0 }).await;
        assert!(matches!(r, ServerMessage::Error { .. }), "{bad}: {r:?}");
        let r = req(&mut ws, &mut n, &ClientMessage::BlobDelete { blob_id: bad.into() }).await;
        assert!(matches!(r, ServerMessage::Error { .. }), "{bad}: {r:?}");
    }
    // Bloco maior que BLOB_CHUNK: recusado.
    let ServerMessage::BlobCreated { blob_id, token } =
        req(&mut ws, &mut n, &ClientMessage::BlobCreate { size: 100 }).await
    else {
        panic!()
    };
    let r = req(&mut ws, &mut n, &ClientMessage::BlobPut { blob_id: blob_id.clone(), token: token.clone(), offset: 0, data: b64(&[1u8; 101]) }).await;
    assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
    let r = req(&mut ws, &mut n, &ClientMessage::BlobPut { blob_id, token, offset: 0, data: "nao e base64!".into() }).await;
    assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn espaco_total_e_reposto_ao_apagar() {
    let dir = tmpdir("espaco");
    let mut c = cfg(dir.clone());
    c.max_total_bytes = 30 << 20;
    let relay = TestRelay::start_with_blobs(fast_cfg(), c).await;
    let (mut ws, mut n) = login(&relay, &SigningKeyPair::generate()).await;
    let ServerMessage::BlobCreated { blob_id, .. } = req(&mut ws, &mut n, &ClientMessage::BlobCreate { size: 20 << 20 }).await else {
        panic!()
    };
    let r = req(&mut ws, &mut n, &ClientMessage::BlobCreate { size: 20 << 20 }).await;
    assert!(matches!(&r, ServerMessage::Error { reason } if reason.contains("espaco")), "{r:?}");
    req(&mut ws, &mut n, &ClientMessage::BlobDelete { blob_id }).await;
    let r = req(&mut ws, &mut n, &ClientMessage::BlobCreate { size: 20 << 20 }).await;
    assert!(matches!(r, ServerMessage::BlobCreated { .. }), "{r:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn disco_quase_cheio_recusa_anexos_novos() {
    let dir = tmpdir("disco");
    let mut c = cfg(dir.clone());
    c.min_free_bytes = u64::MAX / 2; // nenhum disco tem isto livre
    let relay = TestRelay::start_with_blobs(fast_cfg(), c).await;
    let (mut ws, mut n) = login(&relay, &SigningKeyPair::generate()).await;
    let r = req(&mut ws, &mut n, &ClientMessage::BlobCreate { size: 1000 }).await;
    assert!(matches!(&r, ServerMessage::Error { reason } if reason.contains("espaco")), "{r:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn sem_anexos_ativos_o_relay_responde_com_erro() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (mut ws, mut n) = login(&relay, &SigningKeyPair::generate()).await;
    let r = req(&mut ws, &mut n, &ClientMessage::BlobCreate { size: 10 }).await;
    assert!(matches!(&r, ServerMessage::Error { reason } if reason.contains("desativados")), "{r:?}");
    // A ligacao continua a funcionar para o resto.
    let r = req(&mut ws, &mut n, &ClientMessage::FetchPreKeyBundle { user_id: "0".repeat(64) }).await;
    assert!(matches!(r, ServerMessage::PreKeyBundleNotFound), "{r:?}");
}

#[tokio::test]
async fn limpeza_apaga_expirados_e_orfaos_e_nao_mexe_no_resto() {
    let dir = tmpdir("limpeza");
    let mut c = cfg(dir.clone());
    c.ttl = Duration::from_secs(1);
    c.grace = Duration::ZERO;
    let mut store = Store::with_prefix("redis://127.0.0.1:6379", &format!("t-limpeza-{}", rand_u32())).unwrap();
    store.enable_blobs(c).unwrap();
    let owner = "a".repeat(64);
    let (expired, _) = store.blob_create(&owner, 10).await.unwrap();
    std::fs::write(dir.join("LEIA-ME.txt"), b"nao e um anexo").unwrap();
    tokio::time::sleep(Duration::from_millis(2_200)).await; // o Redis expira o registo
    let (fresh, _) = store.blob_create(&owner, 20).await.unwrap();
    let (removed, used) = store.sweep_blobs().await.unwrap();
    assert_eq!(removed, 1);
    assert_eq!(used, 20, "so o ficheiro novo continua reservado");
    assert!(!dir.join(&expired).exists());
    assert!(dir.join(&fresh).exists());
    assert!(dir.join("LEIA-ME.txt").exists(), "ficheiros que nao sao anexos ficam");
    let _ = std::fs::remove_dir_all(dir);
}
