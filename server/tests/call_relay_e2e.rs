//! Chamada de voz de ponta a ponta pelo relay das chamadas (fase 2): dois QtClient
//! (sinalizacao real dentro do ratchet), relay real em localhost (UDP + reserva por
//! WebSocket), pacotes de audio falsos. Sem Redis: o relay das chamadas nao o usa.

use futures_util::{SinkExt, StreamExt};
use qt_ffi::call::CallEvent;
use qt_ffi::*;
use qt_server::call_relay::{serve, CallLimits};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio_tungstenite::tungstenite::Message;

fn tmp(n: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_callrelay_{}_{}_{}.sqlite", n, std::process::id(), id);
    let _ = std::fs::remove_file(&p);
    p
}

struct Peer {
    c: Arc<QtClient>,
    id: PublicIdentity,
}

impl Peer {
    fn new(name: &str, key: u8) -> Peer {
        let c = QtClient::open(tmp(name), vec![key; 32]).unwrap();
        let id = c.create_identity().unwrap();
        Peer { c, id }
    }
    fn cid(&self) -> String {
        self.id.identity_public.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// O relay das mensagens, simulado: entrega a outbox de `from` a `to`.
fn deliver(from: &Peer, to: &Peer) {
    for item in from.c.outbox().unwrap() {
        if item.contact_id == to.cid() {
            to.c.receive_sealed(item.blob.clone()).unwrap();
        }
        from.c.outbox_sent(item.outbox_id).unwrap();
    }
}

fn pair() -> (Peer, Peer) {
    let (a, b) = (Peer::new("a", 0x91), Peer::new("b", 0x92));
    let bundle = peer_bundle_from_relay_bytes(b.c.publishable_bundle().unwrap()).unwrap();
    a.c.start_session(b.id.public_id.clone(), bundle).unwrap();
    a.c.send_text(b.cid(), "olá".into()).unwrap();
    deliver(&a, &b);
    b.c.send_text(a.cid(), "olá!".into()).unwrap();
    deliver(&b, &a);
    deliver(&a, &b);
    (a, b)
}

struct Relay {
    udp: SocketAddr,
    ws: String,
    public: Vec<u8>,
}

async fn relay() -> Relay {
    let keys = qt_transport::generate_static_keypair().unwrap();
    let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let ws = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let r = Relay { udp: udp.local_addr().unwrap(), ws: format!("ws://{}/call", ws.local_addr().unwrap()), public: keys.public };
    serve(udp, Some(ws), &keys.private, CallLimits::default(), Default::default());
    r
}

async fn recv(s: &UdpSocket) -> Vec<u8> {
    let mut buf = vec![0u8; 2048];
    let (n, _) = tokio::time::timeout(Duration::from_secs(2), s.recv_from(&mut buf)).await.expect("nada chegou").unwrap();
    buf.truncate(n);
    buf
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next(ws: &mut Ws) -> Vec<u8> {
    match tokio::time::timeout(Duration::from_secs(2), ws.next()).await.unwrap() {
        Some(Ok(Message::Binary(b))) => b,
        other => panic!("esperava binario: {other:?}"),
    }
}

/// Entra na sala por UDP e devolve "o outro ja esta?".
async fn join_udp(p: &Peer, call: &str, s: &UdpSocket, r: &Relay) -> bool {
    s.send_to(&p.c.call_join_packet(call.into(), r.public.clone()).unwrap(), r.udp).await.unwrap();
    p.c.call_join_ok(call.into(), recv(s).await).unwrap().expect("resposta do relay")
}

/// Liga de a para b; os dois entram na sala ENQUANTO toca (pre-aquecimento); b atende.
async fn call_through_relay(a: &Peer, b: &Peer, r: &Relay) -> (String, UdpSocket, UdpSocket) {
    let call = a.c.call_start(b.cid()).unwrap().call_id;
    deliver(a, b);
    assert!(matches!(b.c.call_events().unwrap()[0], CallEvent::Incoming { .. }));
    let (sa, sb) = (UdpSocket::bind("127.0.0.1:0").await.unwrap(), UdpSocket::bind("127.0.0.1:0").await.unwrap());
    assert!(!join_udp(a, &call, &sa, r).await, "a Alice chega primeiro");
    assert!(join_udp(b, &call, &sb, r).await, "o Bob ve que a Alice ja esta");
    // A Alice e avisada de que o Bob chegou.
    assert_eq!(a.c.call_join_ok(call.clone(), recv(&sa).await).unwrap(), Some(true));
    b.c.call_accept(call.clone()).unwrap();
    deliver(b, a);
    (call, sa, sb)
}

#[tokio::test]
async fn chamada_pelo_relay_udp() {
    let r = relay().await;
    let (a, b) = pair();
    let (call, sa, sb) = call_through_relay(&a, &b, &r).await;
    for i in 0..20u8 {
        sa.send_to(&a.c.call_seal(call.clone(), vec![i; 50]).unwrap(), r.udp).await.unwrap();
        let got = b.c.call_open(call.clone(), recv(&sb).await).unwrap().unwrap();
        assert_eq!(got.frame, vec![i; 50]);
        sb.send_to(&b.c.call_seal(call.clone(), vec![i]).unwrap(), r.udp).await.unwrap();
        assert_eq!(a.c.call_open(call.clone(), recv(&sa).await).unwrap().unwrap().frame, vec![i]);
    }
}

#[tokio::test]
async fn mudar_de_rede_a_meio_da_chamada() {
    let r = relay().await;
    let (a, b) = pair();
    let (call, sa, sb) = call_through_relay(&a, &b, &r).await;
    sa.send_to(&a.c.call_seal(call.clone(), b"wifi".to_vec()).unwrap(), r.udp).await.unwrap();
    assert_eq!(b.c.call_open(call.clone(), recv(&sb).await).unwrap().unwrap().frame, b"wifi");

    // A Alice passa para o 4G: socket novo, sem voltar a entrar na sala.
    let sa4g = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    sa4g.send_to(&a.c.call_seal(call.clone(), b"4g".to_vec()).unwrap(), r.udp).await.unwrap();
    assert_eq!(b.c.call_open(call.clone(), recv(&sb).await).unwrap().unwrap().frame, b"4g");
    // As respostas do Bob vao para o endereco novo.
    sb.send_to(&b.c.call_seal(call.clone(), b"ouves?".to_vec()).unwrap(), r.udp).await.unwrap();
    assert_eq!(a.c.call_open(call.clone(), recv(&sa4g).await).unwrap().unwrap().frame, b"ouves?");
    drop(sa);
}

#[tokio::test]
async fn um_lado_por_udp_outro_pela_reserva_websocket() {
    let r = relay().await;
    let (a, b) = pair();
    let call = a.c.call_start(b.cid()).unwrap().call_id;
    deliver(&a, &b);
    b.c.call_events().unwrap();
    b.c.call_accept(call.clone()).unwrap();
    deliver(&b, &a);

    // A Alice tem UDP; a rede do Bob bloqueia UDP e ele usa o WebSocket.
    let sa = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    assert!(!join_udp(&a, &call, &sa, &r).await);
    let (mut ws, _) = tokio_tungstenite::connect_async(r.ws.as_str()).await.unwrap();
    ws.send(Message::Binary(b.c.call_join_packet(call.clone(), r.public.clone()).unwrap())).await.unwrap();
    assert_eq!(b.c.call_join_ok(call.clone(), next(&mut ws).await).unwrap(), Some(true));
    assert_eq!(a.c.call_join_ok(call.clone(), recv(&sa).await).unwrap(), Some(true));

    sa.send_to(&a.c.call_seal(call.clone(), b"por udp".to_vec()).unwrap(), r.udp).await.unwrap();
    assert_eq!(b.c.call_open(call.clone(), next(&mut ws).await).unwrap().unwrap().frame, b"por udp");
    ws.send(Message::Binary(b.c.call_seal(call.clone(), b"por tcp".to_vec()).unwrap())).await.unwrap();
    assert_eq!(a.c.call_open(call.clone(), recv(&sa).await).unwrap().unwrap().frame, b"por tcp");
}

#[tokio::test]
async fn sala_errada_ou_chave_de_relay_errada_nao_responde() {
    let r = relay().await;
    let (a, b) = pair();
    let call = a.c.call_start(b.cid()).unwrap().call_id;
    let sa = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    // Pedido cifrado para outra chave (relay falso ou pin antigo): o relay ignora.
    let outra = qt_transport::generate_static_keypair().unwrap();
    sa.send_to(&a.c.call_join_packet(call.clone(), outra.public).unwrap(), r.udp).await.unwrap();
    let mut buf = [0u8; 64];
    assert!(tokio::time::timeout(Duration::from_millis(300), sa.recv_from(&mut buf)).await.is_err());
    // Uma resposta forjada (sem a chave das etiquetas) nao e aceite pela app.
    assert_eq!(a.c.call_join_ok(call.clone(), vec![0x21; 18]).unwrap(), None);
}
