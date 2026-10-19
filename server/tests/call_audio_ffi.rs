//! A chamada como a app a usa (fase 4a): QtClient (sinalizacao no ratchet) + CallAudio
//! (motor de audio pela FFI) + relay das chamadas real em localhost. O audio entra e
//! sai como PCM em bytes, igual ao AudioRecord / AudioTrack do Android.

use qt_ffi::call::CallEvent;
use qt_ffi::call_audio::call_frame_bytes;
use qt_ffi::*;
use qt_server::call_relay::{serve, CallLimits};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn tmp(n: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_callaudio_{}_{}_{}.sqlite", n, std::process::id(), id);
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

fn deliver(from: &Peer, to: &Peer) {
    for item in from.c.outbox().unwrap() {
        if item.contact_id == to.cid() {
            to.c.receive_sealed(item.blob.clone()).unwrap();
        }
        from.c.outbox_sent(item.outbox_id).unwrap();
    }
}

/// 20 ms de "voz" em PCM 16 bits little-endian (como o AudioRecord).
fn voice(frame: usize) -> Vec<u8> {
    (0..960)
        .flat_map(|i| {
            let t = (frame * 960 + i) as f64 / 48_000.0;
            let env = (0.5 + 0.5 * (t * 2.0 * std::f64::consts::PI * 3.0).sin()).powi(2);
            let mut v = 0.0;
            for h in 1..10 {
                v += (t * 2.0 * std::f64::consts::PI * 150.0 * h as f64).sin() / h as f64;
            }
            ((v * env * 6000.0) as i16).to_le_bytes()
        })
        .collect()
}

fn energy(pcm: &[u8]) -> f64 {
    pcm.as_chunks::<2>().0.iter().map(|b| (i16::from_le_bytes(*b) as f64).powi(2)).sum::<f64>() / (pcm.len() / 2) as f64
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chamada_como_na_app() {
    let keys = qt_transport::generate_static_keypair().unwrap();
    let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let ws = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws.local_addr().unwrap().port();
    let udp_port = udp.local_addr().unwrap().port();
    serve(udp, Some(ws), &keys.private, CallLimits::default());
    // O motor deriva o UDP (porta 443) do URL; aqui o relay local esta noutra porta, por
    // isso o UDP nao responde e o motor passa para a reserva ws://127.0.0.1:<porta>/call.
    let _ = udp_port;
    let relay_url = format!("ws://127.0.0.1:{ws_port}");
    let pin = keys.public.clone();

    let (a, b) = (Peer::new("a", 0xA5), Peer::new("b", 0xB6));
    tokio::task::spawn_blocking(move || {
        let bundle = peer_bundle_from_relay_bytes(b.c.publishable_bundle().unwrap()).unwrap();
        a.c.start_session(b.id.public_id.clone(), bundle).unwrap();
        a.c.send_text(b.cid(), "olá".into()).unwrap();
        deliver(&a, &b);
        b.c.send_text(a.cid(), "olá!".into()).unwrap();
        deliver(&b, &a);
        deliver(&a, &b);

        // A Alice liga e arranca logo o audio (entra na sala enquanto toca).
        let call = a.c.call_start(b.cid()).unwrap().call_id;
        let audio_a = a.c.call_audio_start(call.clone(), relay_url.clone(), pin.clone()).unwrap();
        deliver(&a, &b);
        assert!(matches!(b.c.call_events().unwrap()[0], CallEvent::Incoming { .. }));
        let audio_b = b.c.call_audio_start(call.clone(), relay_url.clone(), pin.clone()).unwrap();
        // Chamar outra vez devolve o mesmo motor.
        let again = b.c.call_audio_start(call.clone(), relay_url.clone(), pin.clone()).unwrap();
        assert!(Arc::ptr_eq(&audio_b, &again));
        assert!(!audio_a.active() && !audio_b.active());

        let t = Instant::now();
        while !(audio_a.stats().joined && audio_b.stats().joined) {
            assert!(t.elapsed() < Duration::from_secs(6), "nao entraram: {:?} {:?}", audio_a.stats(), audio_b.stats());
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(audio_a.stats().transport, "websocket");

        // O Bob atende: as chaves passam para os motores dos dois.
        b.c.call_accept(call.clone()).unwrap();
        deliver(&b, &a);
        assert!(a.c.call_events().unwrap().contains(&CallEvent::Answered { call_id: call.clone() }));
        assert!(audio_a.active() && audio_b.active());

        assert!(audio_a.push_capture(vec![0u8; 10]).is_err(), "tamanho errado");
        let t0 = Instant::now();
        let mut heard = Vec::new();
        for i in 0..80 {
            audio_a.push_capture(voice(i)).unwrap();
            let out = audio_b.pull_playback();
            assert_eq!(out.len(), call_frame_bytes() as usize);
            heard.push(energy(&out));
            let next = t0 + Duration::from_millis(20 * (i as u64 + 1));
            if let Some(d) = next.checked_duration_since(Instant::now()) {
                std::thread::sleep(d);
            }
        }
        let audible = heard[20..].iter().filter(|e| **e > 1e4).count();
        assert!(audible >= 45, "so {audible} de 60 frames com som; {:?}", audio_b.stats());

        // Desligar para o motor dos dois lados.
        a.c.call_hangup(call.clone()).unwrap();
        deliver(&a, &b);
        assert!(!audio_a.active());
        assert!(!audio_b.active());
        assert_eq!(audio_b.pull_playback(), vec![0u8; call_frame_bytes() as usize]);
    })
    .await
    .unwrap();
}
