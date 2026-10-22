//! Copia de recuperacao (`BackupPut` / `BackupGet` / `BackupDelete`): partes, troca atomica,
//! dono, palavras novas e conta apagada. Precisa de um Redis em 127.0.0.1:6379.

mod common;
use common::*;

use base64::Engine;
use qt_core::primitives::SigningKeyPair;
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::protocol::*;
use qt_server::ServerConfig;
use qt_transport::generate_static_keypair;

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

async fn client(relay: &TestRelay, s: &SigningKeyPair) -> RelayClient {
    let n = generate_static_keypair().unwrap();
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    RelayClient::connect(&relay.url, n.private, s.to_bytes(), cfg).await.unwrap()
}

async fn put(c: &mut RelayClient, lookup: &str, gen: &str, chunks: &[Vec<u8>]) -> ServerMessage {
    let mut last = ServerMessage::Ack;
    for (i, d) in chunks.iter().enumerate() {
        last = c
            .request(ClientMessage::BackupPut {
                lookup: lookup.into(),
                generation: gen.into(),
                part: i as u32,
                parts: chunks.len() as u32,
                data: b64(d),
            })
            .await
            .unwrap();
        if !matches!(last, ServerMessage::Ack) {
            return last;
        }
    }
    last
}

async fn get_all(c: &mut RelayClient, lookup: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut part = 0;
    loop {
        match c.request(ClientMessage::BackupGet { lookup: lookup.into(), part }).await.unwrap() {
            ServerMessage::BackupPart { part: p, parts, data } => {
                assert_eq!(p, part);
                out.extend(base64::engine::general_purpose::STANDARD.decode(data).unwrap());
                part += 1;
                if part == parts {
                    return Some(out);
                }
            }
            ServerMessage::BackupNotFound => return None,
            other => panic!("{other:?}"),
        }
    }
}

const L1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const L2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

#[tokio::test]
async fn copia_em_partes_e_lida_por_outra_ligacao_e_substituida_inteira() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let alice = SigningKeyPair::generate();
    let mut a = client(&relay, &alice).await;
    a.register_bundle(valid_bundle(&alice, 1)).await.unwrap();

    let parts = vec![vec![1u8; BACKUP_PART_MAX], vec![2u8; BACKUP_PART_MAX], vec![3u8; 100]];
    assert!(matches!(put(&mut a, L1, "aaaaaaaaaaaaaaaa", &parts).await, ServerMessage::Ack));

    // Num telemovel novo: outra identidade (temporaria, sem registo) le a copia.
    let mut fresh = client(&relay, &SigningKeyPair::generate()).await;
    assert_eq!(get_all(&mut fresh, L1).await.unwrap(), parts.concat());
    assert_eq!(get_all(&mut fresh, L2).await, None);

    // Copia nova (outra geracao): substitui a anterior inteira.
    assert!(matches!(put(&mut a, L1, "bbbbbbbbbbbbbbbb", &[vec![9u8; 10]]).await, ServerMessage::Ack));
    assert_eq!(get_all(&mut fresh, L1).await.unwrap(), vec![9u8; 10]);

    // A meio de um envio (so a 1.a de 2 partes): continua a valer a copia anterior.
    let r = a
        .request(ClientMessage::BackupPut { lookup: L1.into(), generation: "cccccccccccccccc".into(), part: 0, parts: 2, data: b64(&[7u8; 5]) })
        .await
        .unwrap();
    assert!(matches!(r, ServerMessage::Ack));
    assert_eq!(get_all(&mut fresh, L1).await.unwrap(), vec![9u8; 10]);
    // Ultima parte sem as outras: recusada.
    let r = a
        .request(ClientMessage::BackupPut { lookup: L1.into(), generation: "dddddddddddddddd".into(), part: 1, parts: 2, data: b64(&[7u8; 5]) })
        .await
        .unwrap();
    assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
}

#[tokio::test]
async fn so_o_dono_substitui_e_sem_registo_nao_se_guarda() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let (alice, eve) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut a = client(&relay, &alice).await;
    let mut e = client(&relay, &eve).await;

    // Sem bundle registado: recusado.
    let r = put(&mut a, L1, "aaaaaaaaaaaaaaaa", &[vec![1u8; 4]]).await;
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == NOT_REGISTERED), "{r:?}");

    a.register_bundle(valid_bundle(&alice, 2)).await.unwrap();
    e.register_bundle(valid_bundle(&eve, 3)).await.unwrap();
    assert!(matches!(put(&mut a, L1, "aaaaaaaaaaaaaaaa", &[vec![1u8; 4]]).await, ServerMessage::Ack));
    let r = put(&mut e, L1, "eeeeeeeeeeeeeeee", &[vec![6u8; 4]]).await;
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == BACKUP_NOT_OWNER), "{r:?}");
    assert_eq!(get_all(&mut e, L1).await.unwrap(), vec![1u8; 4]);

    // Formatos invalidos.
    for bad in [
        ClientMessage::BackupPut { lookup: "xyz".into(), generation: "aaaaaaaaaaaaaaaa".into(), part: 0, parts: 1, data: b64(&[1]) },
        ClientMessage::BackupPut { lookup: L1.into(), generation: "aa".into(), part: 0, parts: 1, data: b64(&[1]) },
        ClientMessage::BackupPut { lookup: L1.into(), generation: "aaaaaaaaaaaaaaaa".into(), part: 0, parts: 9, data: b64(&[1]) },
        ClientMessage::BackupPut { lookup: L1.into(), generation: "aaaaaaaaaaaaaaaa".into(), part: 1, parts: 1, data: b64(&[1]) },
        ClientMessage::BackupPut { lookup: L1.into(), generation: "aaaaaaaaaaaaaaaa".into(), part: 0, parts: 1, data: "!!".into() },
        ClientMessage::BackupPut { lookup: L1.into(), generation: "aaaaaaaaaaaaaaaa".into(), part: 0, parts: 1, data: b64(&vec![0u8; BACKUP_PART_MAX + 1]) },
    ] {
        let r = a.request(bad).await.unwrap();
        assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
    }
}

#[tokio::test]
async fn palavras_novas_apagam_a_copia_antiga_e_apagar_a_conta_apaga_a_copia() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let alice = SigningKeyPair::generate();
    let mut a = client(&relay, &alice).await;
    a.register_bundle(valid_bundle(&alice, 4)).await.unwrap();
    assert!(matches!(put(&mut a, L1, "aaaaaaaaaaaaaaaa", &[vec![1u8; 4]]).await, ServerMessage::Ack));

    // Palavras novas (outro lookup): a copia antiga desaparece.
    assert!(matches!(put(&mut a, L2, "aaaaaaaaaaaaaaaa", &[vec![2u8; 4]]).await, ServerMessage::Ack));
    assert_eq!(get_all(&mut a, L1).await, None);
    assert_eq!(get_all(&mut a, L2).await.unwrap(), vec![2u8; 4]);

    // BackupDelete apaga; voltar a guardar funciona.
    assert!(matches!(a.request(ClientMessage::BackupDelete).await.unwrap(), ServerMessage::Ack));
    assert_eq!(get_all(&mut a, L2).await, None);
    assert!(matches!(put(&mut a, L2, "bbbbbbbbbbbbbbbb", &[vec![3u8; 4]]).await, ServerMessage::Ack));

    // Apagar a conta apaga a copia.
    a.delete_account().await.unwrap();
    assert_eq!(get_all(&mut a, L2).await, None);
}
