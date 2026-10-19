//! App 0.9.3: prova de trabalho para contas novas, one-time prekeys servidas pelo
//! relay (uma por pedido) e limites de pedidos por conta. Precisa de Redis local.

mod common;
use common::*;

use qt_core::primitives::SigningKeyPair;
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::limits::Rate;
use qt_server::protocol::*;
use qt_server::ServerConfig;
use qt_transport::generate_static_keypair;

fn uid(s: &SigningKeyPair) -> String {
    user_id_from_signing_public(&s.verifying_key.to_bytes())
}

async fn client(relay: &TestRelay, s: &SigningKeyPair) -> RelayClient {
    let n = generate_static_keypair().unwrap();
    let cfg = RelayConfig { max_retries: 0, ..RelayConfig::default() };
    RelayClient::connect(&relay.url, n.private, s.to_bytes(), cfg).await.unwrap()
}

fn register(s: &SigningKeyPair, pow: Option<u64>, keys: Option<Vec<Vec<u8>>>) -> ClientMessage {
    ClientMessage::RegisterPreKeyBundle { user_id: uid(s), bundle_bytes: valid_bundle(s, 1), pow, one_time_pre_keys: keys }
}

fn otpk(id: u32) -> Vec<u8> {
    let mut k = id.to_be_bytes().to_vec();
    k.extend_from_slice(&[id as u8; 32]);
    k
}

#[tokio::test]
async fn conta_nova_so_com_prova_de_trabalho() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let s = SigningKeyPair::generate();
    let mut c = client(&relay, &s).await;
    // Sem prova, ou com a prova de outra conta: recusado.
    let r = c.request(register(&s, None, None)).await.unwrap();
    assert!(matches!(r, ServerMessage::Error { ref reason } if reason == POW_REQUIRED), "{r:?}");
    let other = SigningKeyPair::generate();
    let wrong = qt_core::pow::pow_solve(&other.verifying_key.to_bytes(), qt_core::pow::POW_BITS);
    // (por acaso pode valer para as duas: com 7 bits e 1 em 128; repete com outra se for o caso)
    if !qt_core::pow::pow_verify(&s.verifying_key.to_bytes(), wrong.unwrap(), qt_core::pow::POW_BITS) {
        let r = c.request(register(&s, wrong, None)).await.unwrap();
        assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
    }
    // Com prova valida: aceite. Depois, a conta ja existe e nao precisa de prova (apps antigas).
    let pow = qt_core::pow::pow_solve(&s.verifying_key.to_bytes(), qt_core::pow::POW_BITS);
    assert!(matches!(c.request(register(&s, pow, None)).await.unwrap(), ServerMessage::Ack));
    assert!(matches!(c.request(register(&s, None, None)).await.unwrap(), ServerMessage::Ack));
}

#[tokio::test]
async fn uma_one_time_prekey_por_pedido_e_o_lote_novo_substitui() {
    let relay = TestRelay::start(ServerConfig::default()).await;
    let (bob, alice) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut b = client(&relay, &bob).await;
    let pow = qt_core::pow::pow_solve(&bob.verifying_key.to_bytes(), qt_core::pow::POW_BITS);
    let r = b.request(register(&bob, pow, Some(vec![otpk(1), otpk(2)]))).await.unwrap();
    assert!(matches!(r, ServerMessage::Ack), "{r:?}");
    let mut a = client(&relay, &alice).await;
    let fetch = || ClientMessage::FetchPreKeyBundle { user_id: uid(&bob) };
    let mut got = vec![];
    for _ in 0..3 {
        match a.request(fetch()).await.unwrap() {
            ServerMessage::PreKeyBundle { one_time_pre_key, .. } => got.push(one_time_pre_key),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(got, vec![Some(otpk(1)), Some(otpk(2)), None], "cada uma so uma vez, depois nenhuma");
    // Lote novo: substitui (nao acumula).
    assert!(matches!(b.request(register(&bob, None, Some(vec![otpk(7)]))).await.unwrap(), ServerMessage::Ack));
    match a.request(fetch()).await.unwrap() {
        ServerMessage::PreKeyBundle { one_time_pre_key, .. } => assert_eq!(one_time_pre_key, Some(otpk(7))),
        other => panic!("{other:?}"),
    }
    // Lixo: chave com tamanho errado, numero 0, ou lote grande demais.
    for bad in [vec![vec![1u8; 10]], vec![otpk(0)], (1..=(MAX_OTPKS as u32 + 1)).map(otpk).collect()] {
        let r = b.request(register(&bob, None, Some(bad))).await.unwrap();
        assert!(matches!(r, ServerMessage::Error { .. }), "{r:?}");
    }
}

#[tokio::test]
async fn limite_de_envios_e_de_bundles_por_conta() {
    let cfg = ServerConfig {
        send_rate: Rate { burst: 5, per_minute: 1 },
        fetch_bundle_rate: Rate { burst: 2, per_minute: 1 },
        ..ServerConfig::default()
    };
    let relay = TestRelay::start(cfg).await;
    let (a, b) = (SigningKeyPair::generate(), SigningKeyPair::generate());
    let mut ca = client(&relay, &a).await;
    for i in 0..5 {
        assert!(ca.send_message(&uid(&b), vec![], vec![i; 8]).await.is_ok(), "envio {i}");
    }
    let r = ca.send_message(&uid(&b), vec![], vec![9; 8]).await;
    assert!(matches!(r, Err(qt_server::client::RelayError::Rejected(ref s)) if s == RATE_LIMITED), "{r:?}");
    // Outra conta nao e afetada.
    let mut cb = client(&relay, &b).await;
    assert!(cb.send_message(&uid(&a), vec![], vec![1; 8]).await.is_ok());
    let fetch = || ClientMessage::FetchPreKeyBundle { user_id: uid(&b) };
    for _ in 0..2 {
        assert!(!matches!(ca.request(fetch()).await.unwrap(), ServerMessage::Error { .. }));
    }
    assert!(matches!(ca.request(fetch()).await.unwrap(), ServerMessage::Error { ref reason } if reason == RATE_LIMITED));
}
