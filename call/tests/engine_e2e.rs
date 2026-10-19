//! Duas pontas de uma chamada (motores completos: Opus, cifra, jitter buffer, rede)
//! atraves do relay das chamadas real (qt_server::call_relay) em localhost.
//! Corre em tempo real (20 ms por frame), uns segundos por teste.

use qt_call::codec::{FRAME_SAMPLES, SAMPLE_RATE};
use qt_call::engine::{join_packet, CallEngine, EngineConfig, RelayTarget, Transport};
use qt_core::call::{CallKeys, MediaSession, Role, RoomKeys};
use qt_server::call_relay::{serve, CallLimits};
use std::time::{Duration, Instant};

fn speech_like(frame: usize) -> Vec<i16> {
    (0..FRAME_SAMPLES)
        .map(|i| {
            let t = (frame * FRAME_SAMPLES + i) as f64 / SAMPLE_RATE as f64;
            let f0 = 140.0 + 30.0 * (t * 2.0 * std::f64::consts::PI * 0.7).sin();
            let env = (0.5 + 0.5 * (t * 2.0 * std::f64::consts::PI * 3.0).sin()).powi(2);
            let mut v = 0.0;
            for h in 1..12 {
                v += (t * 2.0 * std::f64::consts::PI * f0 * h as f64).sin() / h as f64;
            }
            (v * env * 6000.0) as i16
        })
        .collect()
}

fn energy(x: &[i16]) -> f64 {
    x.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / x.len() as f64
}

struct Relay {
    target: RelayTarget,
    public: Vec<u8>,
}

async fn relay() -> Relay {
    let keys = qt_transport::generate_static_keypair().unwrap();
    let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let ws = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = RelayTarget {
        udp: udp.local_addr().unwrap().to_string(),
        ws_url: Some(format!("ws://{}/call", ws.local_addr().unwrap())),
    };
    serve(udp, Some(ws), &keys.private, CallLimits::default());
    Relay { target, public: keys.public }
}

/// Os dois lados de uma chamada ja atendida (segredos aleatorios, como na fase 1).
fn sessions() -> (MediaSession, MediaSession, RoomKeys) {
    let rnd = |n: u8| -> [u8; 32] {
        let mut b = [n; 32];
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos().to_le_bytes();
        b[..16].copy_from_slice(&t);
        b
    };
    let (offer, answer) = (rnd(1), rnd(2));
    let id: [u8; 16] = rnd(3)[..16].try_into().unwrap();
    let a = MediaSession::new(CallKeys::derive(&offer, &answer, &id, Role::Caller), 128).unwrap();
    let b = MediaSession::new(CallKeys::derive(&offer, &answer, &id, Role::Callee), 128).unwrap();
    (a, b, RoomKeys::derive(&offer, &id))
}

fn start(r: &Relay, media: MediaSession, role: Role, room: &RoomKeys, cfg: impl FnOnce(&mut EngineConfig)) -> CallEngine {
    let mut c = EngineConfig::new(r.target.clone());
    cfg(&mut c);
    CallEngine::start(c, media, join_packet(&r.public, role, room).unwrap()).unwrap()
}

fn wait_joined(e: &CallEngine) {
    let t = Instant::now();
    while !e.stats().joined {
        assert!(t.elapsed() < Duration::from_secs(5), "nao entrou na sala: {:?}", e.stats());
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Fala `frames` x 20 ms para `a` e ouve em `b`, em tempo real. Devolve a energia de
/// cada frame tocado em `b`.
fn talk(a: &CallEngine, b: &CallEngine, frames: usize) -> Vec<f64> {
    let mut heard = Vec::new();
    let t0 = Instant::now();
    let mut out = vec![0i16; FRAME_SAMPLES];
    for i in 0..frames {
        a.push_capture(&speech_like(i)).unwrap();
        b.pull_playback(&mut out).unwrap();
        heard.push(energy(&out));
        let next = t0 + Duration::from_millis(20 * (i as u64 + 1));
        if let Some(d) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(d);
        }
    }
    heard
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chamada_completa_por_udp() {
    let r = relay().await;
    let (ma, mb, room) = sessions();
    let a = start(&r, ma, Role::Caller, &room, |_| {});
    let b = start(&r, mb, Role::Callee, &room, |_| {});
    let (a, b) = tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        let heard = talk(&a, &b, 100);
        // Depois do arranque (atraso do jitter buffer), ouve-se voz quase sempre.
        let audible = heard[20..].iter().filter(|e| **e > 1e4).count();
        assert!(audible >= 60, "so {audible} de 80 frames com som; {:?}", b.stats());
        (a, b)
    })
    .await
    .unwrap();
    let (sa, sb) = (a.stats(), b.stats());
    eprintln!("udp: A {sa:?}\n     B {sb:?}");
    assert_eq!(sa.transport, Transport::Udp);
    assert!(sa.relay_rtt_ms.is_some());
    assert!(sb.received >= 95, "{sb:?}");
    assert_eq!(sb.invalid, 0);
    assert!(sa.avg_opus_bytes > 30 && sa.avg_opus_bytes <= 126, "{sa:?}");
    assert!(sb.jitter.target_ms <= 100, "{sb:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn perdas_sao_reconstruidas_com_dred() {
    let r = relay().await;
    let (ma, mb, room) = sessions();
    let a = start(&r, ma, Role::Caller, &room, |_| {});
    let b = start(&r, mb, Role::Callee, &room, |c| c.simulated_loss_percent = 15);
    let b = tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        let heard = talk(&a, &b, 150);
        let audible = heard[20..].iter().filter(|e| **e > 1e4).count();
        assert!(audible >= 90, "so {audible} de 130 frames com som; {:?}", b.stats());
        b
    })
    .await
    .unwrap();
    let s = b.stats();
    eprintln!("perdas simuladas: {s:?}");
    assert!(s.simulated_drops > 5, "{s:?}");
    assert!(s.dred_frames > 0, "o DRED nunca foi usado: {s:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reserva_websocket() {
    let r = relay().await;
    let (ma, mb, room) = sessions();
    let a = start(&r, ma, Role::Caller, &room, |_| {});
    let b = start(&r, mb, Role::Callee, &room, |c| c.force_ws = true);
    let b = tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        let heard = talk(&a, &b, 80);
        let audible = heard[20..].iter().filter(|e| **e > 1e4).count();
        assert!(audible >= 45, "so {audible} de 60 frames com som; {:?}", b.stats());
        b
    })
    .await
    .unwrap();
    assert_eq!(b.stats().transport, Transport::WebSocket);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn entra_na_sala_antes_de_atender() {
    let r = relay().await;
    let (ma, mb, room) = sessions();
    // Os dois entram enquanto toca, sem as chaves do audio.
    let mk = |role| {
        let c = EngineConfig::new(r.target.clone());
        CallEngine::start_pending(c, room.clone(), join_packet(&r.public, role, &room).unwrap()).unwrap()
    };
    let (a, b) = (mk(Role::Caller), mk(Role::Callee));
    let b = tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        assert!(a.stats().peer_present && b.stats().peer_present);
        // Antes de atender, o microfone nao sai.
        a.push_capture(&speech_like(0)).unwrap();
        assert_eq!(a.stats().sent, 0);
        // Atendem: o audio passa logo.
        a.attach_media(ma);
        b.attach_media(mb);
        let heard = talk(&a, &b, 60);
        let audible = heard[20..].iter().filter(|e| **e > 1e4).count();
        assert!(audible >= 30, "so {audible} de 40 frames com som; {:?}", b.stats());
        b
    })
    .await
    .unwrap();
    assert!(b.stats().received >= 55);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn udp_bloqueado_passa_para_a_reserva() {
    let r = relay().await;
    let (_, mb, room) = sessions();
    // UDP para um sitio que nao responde: aos 2 s tenta o WebSocket.
    let mut target = r.target.clone();
    target.udp = "127.0.0.1:9".into();
    let relay2 = Relay { target, public: r.public.clone() };
    let b = start(&relay2, mb, Role::Callee, &room, |_| {});
    let b = tokio::task::spawn_blocking(move || {
        wait_joined(&b);
        b
    })
    .await
    .unwrap();
    assert_eq!(b.stats().transport, Transport::WebSocket);
}
