//! Duas pontas de uma chamada (motores completos: Opus, cifra, jitter buffer, rede)
//! atraves do relay das chamadas real (qt_server::call_relay) em localhost.
//! Corre em tempo real (20 ms por frame), uns segundos por teste.

use qt_call::codec::{FRAME_SAMPLES, SAMPLE_RATE};
use qt_call::engine::{join_packet, CallEngine, EngineConfig, RelayTarget, Transport};
use qt_core::call::{CallKeys, MediaSession, Role, RoomKeys};
use qt_server::call_relay::{serve, CallLimits};
use std::net::SocketAddr;
use std::sync::Arc;
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

/// Os dois falam ao mesmo tempo (como numa chamada real, sem DTX: 50 pacotes/s em cada
/// sentido). Devolve a energia do que `b` ouviu.
fn talk_both(a: &CallEngine, b: &CallEngine, frames: usize) -> Vec<f64> {
    let mut heard = Vec::new();
    let t0 = Instant::now();
    let mut out = vec![0i16; FRAME_SAMPLES];
    for i in 0..frames {
        a.push_capture(&speech_like(i)).unwrap();
        b.push_capture(&speech_like(i + 7)).unwrap();
        b.pull_playback(&mut out).unwrap();
        heard.push(energy(&out));
        a.pull_playback(&mut out).unwrap();
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

/// Frames inventados (PLC/DRED) ou abandonados no outro lado quando o direto cai: nos dois
/// sentidos (medido: 10-11, com 7 pacotes perdidos) e so num (medido: 22, com 13 perdidos).
/// Limites = medido + margem: protegem o ganho das sondas a 100 ms e da falta de audio como
/// sinal (antes: ~37 frames, 0,75 s).
const CUT_MAX: u64 = 18;
const CUT_ONE_WAY_MAX: u64 = 25;

/// Espera ate `f` ser verdade (ou falha ao fim de `secs`).
fn wait_for(secs: u64, what: &str, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(secs), "{what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ligacao_direta_e_volta_ao_relay_sem_cortar() {
    let r = relay().await;
    let (ma, mb, room) = sessions();
    let a = start(&r, ma, Role::Caller, &room, |c| c.p2p_allow_loopback = true);
    let b = start(&r, mb, Role::Callee, &room, |c| c.p2p_allow_loopback = true);
    let (a, b) = tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        // O nucleo liga a ligacao direta dos dois lados e troca os enderecos (no ratchet).
        a.p2p_enable();
        b.p2p_enable();
        wait_for(5, "enderecos de A", || a.p2p_local_candidates().is_some_and(|l| !l.is_empty()));
        wait_for(5, "enderecos de B", || b.p2p_local_candidates().is_some_and(|l| !l.is_empty()));
        a.p2p_set_peer(b.p2p_local_candidates().unwrap());
        b.p2p_set_peer(a.p2p_local_candidates().unwrap());
        wait_for(5, "caminho direto", || a.stats().direct && b.stats().direct);
        let heard = talk(&a, &b, 100);
        let audible = heard[20..].iter().filter(|e| **e > 1e4).count();
        assert!(audible >= 60, "direto: so {audible} de 80 frames com som; {:?}", b.stats());
        let sb = b.stats();
        assert!(sb.received_direct >= 90, "o audio nao veio direto: {sb:?}");
        assert_eq!(sb.invalid, 0, "{sb:?}");
        // Com o caminho bom ja so vai direto (o primeiro segundo foi em duplicado).
        assert!(!a.stats().both_paths, "{:?}", a.stats());

        // O caminho direto morre nos dois sentidos (aqui: B deixa de responder, de aceitar e de
        // enviar por la). A fala continua SEM esperar os 3 s: A da pela falta do audio de B
        // (150 ms) ou das respostas as sondas (250 ms) e envia pelos dois caminhos.
        let _ = talk_both(&a, &b, 30);
        let (before, s0) = (b.stats().received, b.stats());
        b.p2p_disable();
        let heard = talk_both(&a, &b, 100);
        let s1 = b.stats();
        let audible = heard.iter().filter(|e| **e > 1e4).count();
        // O corte e medido diretamente: frames que B teve de inventar (PLC) ou esperar.
        let concealed = (s1.plc_frames - s0.plc_frames) + (s1.dred_frames - s0.dred_frames);
        let (holds, gave_up) = (s1.jitter.holds - s0.jitter.holds, s1.jitter.gave_up - s0.jitter.gave_up);
        eprintln!("queda: som {audible}/100, recebidos {}, inventados {concealed}, esperas {holds}, desistencias {gave_up}", s1.received - before);
        // Ate A dar pela falta do audio de B pelo direto (150 ms) perde-se o que foi so por la.
        // MEDIDO: ver o relatorio do teste. O limite protege o ganho.
        assert!(concealed + gave_up <= CUT_MAX, "corte de {} frames: {s1:?}", concealed + gave_up);
        assert!(s1.received - before >= 65, "{s1:?}");
        // Aos 3 s sem respostas, A fica so no relay; a chamada continua.
        wait_for(6, "A devia ter ficado so no relay", || !a.stats().direct);
        let before = b.stats().received;
        let heard = talk(&a, &b, 80);
        let audible = heard[20..].iter().filter(|e| **e > 1e4).count();
        assert!(audible >= 45, "relay: so {audible} de 60 frames com som; {:?}", b.stats());
        assert!(b.stats().received - before >= 70, "{:?}", b.stats());
        (a, b)
    })
    .await
    .unwrap();
    eprintln!("p2p: A {:?}\n     B {:?}", a.stats(), b.stats());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sem_ligacao_direta_nada_sai_para_o_outro_telemovel() {
    let r = relay().await;
    let (ma, mb, room) = sessions();
    // Um "outro telemovel" falso: nao pode receber nada.
    let spy = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    spy.set_read_timeout(Some(Duration::from_millis(1500))).unwrap();
    let spy_addr = spy.local_addr().unwrap();
    let mk = |role| {
        let mut c = EngineConfig::new(r.target.clone());
        c.p2p_allow_loopback = true;
        CallEngine::start_pending(c, room.clone(), join_packet(&r.public, role, &room).unwrap()).unwrap()
    };
    let (a, b) = (mk(Role::Caller), mk(Role::Callee));
    tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        // Antes de atender: ligar a ligacao direta nao faz nada.
        a.p2p_enable();
        a.p2p_set_peer(vec![spy_addr]);
        // Atendida mas sem `p2p_enable` (nao verificado ou "sempre pelo relay"): nada.
        b.attach_media(mb);
        b.p2p_set_peer(vec![spy_addr]);
        a.attach_media(ma);
        a.p2p_set_peer(vec![spy_addr]);
        let _ = talk(&a, &b, 30);
        let mut buf = [0u8; 64];
        assert!(spy.recv_from(&mut buf).is_err(), "um endereco saiu para fora do relay");
        assert!(a.p2p_local_candidates().is_none() && b.p2p_local_candidates().is_none());
        assert!(!a.stats().direct && !b.stats().direct);
        assert!(b.stats().received > 0, "o audio pelo relay continua: {:?}", b.stats());
    })
    .await
    .unwrap();
}

/// Proxy UDP entre um motor e o relay. Atrasa o que vai para o relay (`up`: a copia pelo
/// relay chega mais tarde do que a direta) e pode deitar fora um tipo de pacote
/// (`drop_kind`: um relay antigo nao responde ao pedido de endereco 0x22).
async fn proxy(relay: &str, up: Duration, drop_kind: Option<u8>) -> String {
    let front = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let back = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    back.connect(relay).await.unwrap();
    let client = Arc::new(std::sync::Mutex::new(None::<SocketAddr>));
    {
        let (front, back, client) = (front.clone(), back.clone(), client.clone());
        tokio::spawn(async move {
            let mut buf = vec![0u8; 2048];
            loop {
                let Ok((n, from)) = front.recv_from(&mut buf).await else { continue };
                *client.lock().unwrap() = Some(from);
                if n == 0 || drop_kind == Some(buf[0]) {
                    continue;
                }
                let (data, back) = (buf[..n].to_vec(), back.clone());
                tokio::spawn(async move {
                    tokio::time::sleep(up).await;
                    let _ = back.send(&data).await;
                });
            }
        });
    }
    {
        let front2 = front.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 2048];
            loop {
                let Ok(n) = back.recv(&mut buf).await else { continue };
                let to = *client.lock().unwrap();
                if let Some(c) = to {
                    let _ = front2.send_to(&buf[..n], c).await;
                }
            }
        });
    }
    front.local_addr().unwrap().to_string()
}

/// Liga a ligacao direta dos dois lados e troca os enderecos (o que o nucleo faz pelo ratchet).
fn go_direct(a: &CallEngine, b: &CallEngine) {
    a.p2p_enable();
    b.p2p_enable();
    wait_for(5, "enderecos de A", || a.p2p_local_candidates().is_some_and(|l| !l.is_empty()));
    wait_for(5, "enderecos de B", || b.p2p_local_candidates().is_some_and(|l| !l.is_empty()));
    a.p2p_set_peer(b.p2p_local_candidates().unwrap());
    b.p2p_set_peer(a.p2p_local_candidates().unwrap());
    wait_for(5, "caminho direto", || a.stats().direct && b.stats().direct);
}

/// Sondas que `e` envia durante `secs` segundos.
fn probes_in(e: &CallEngine, secs: u64) -> u64 {
    let before = e.stats().probes_sent;
    std::thread::sleep(Duration::from_secs(secs));
    e.stats().probes_sent - before
}

/// (1) JANELA: a copia pelo relay chega ~150 ms (7-8 pacotes) depois da direta. Quando o
/// direto perde um pacote, a copia do relay e aceite (nao "demasiado antiga").
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copia_pelo_relay_preenche_o_que_o_direto_perdeu() {
    let r = relay().await;
    let slow = Relay { target: RelayTarget { udp: proxy(&r.target.udp, Duration::from_millis(150), None).await, ws_url: None }, public: r.public.clone() };
    let (ma, mb, room) = sessions();
    let a = start(&slow, ma, Role::Caller, &room, |c| {
        c.p2p_allow_loopback = true;
        c.test_both_paths = true;
    });
    let b = start(&r, mb, Role::Callee, &room, |c| {
        c.p2p_allow_loopback = true;
        c.simulated_direct_loss_percent = 30;
    });
    let (a, b) = tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        go_direct(&a, &b);
        let (sent0, s0) = (a.stats().sent, b.stats());
        let heard = talk(&a, &b, 200);
        std::thread::sleep(Duration::from_millis(400)); // as ultimas copias pelo relay
        let (sent, sb) = (a.stats().sent - sent0, b.stats());
        let (got, lost_direct, dups) = (sb.received - s0.received, sb.simulated_drops - s0.simulated_drops, sb.duplicates - s0.duplicates);
        eprintln!("janela: enviados={sent} aceites={got} perdidos_no_direto={lost_direct} copias={dups} B {sb:?}");
        assert!(lost_direct >= 30, "o teste nao perdeu pacotes no direto: {sb:?}");
        // Todos os pacotes chegaram uma vez: os que o direto perdeu vieram pelo relay.
        assert!(got + 2 >= sent, "so {got} de {sent} aceites (copias do relay recusadas?): {sb:?}");
        assert_eq!(got + dups, 2 * sent - lost_direct, "cada pacote: 1 aceite, o resto copias");
        assert_eq!(sb.invalid, 0, "{sb:?}");
        // O que o direto perdeu e a copia trouxe tarde demais para tocar (o jitter buffer
        // esta a 40 ms e o relay a +150 ms) e reconstruido pelo DRED: ouve-se na mesma.
        let audible = heard[20..].iter().filter(|e| **e > 1e4).count();
        assert!(audible >= 120, "so {audible} de 180 frames com som; {sb:?}");
        (a, b)
    })
    .await
    .unwrap();
    drop((a, b));
}

/// (2) ESTATISTICAS: o mesmo audio pelos dois caminhos. Fica a primeira copia; a segunda
/// nem chega ao jitter buffer: nao conta como recebida, atrasada nem perdida.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copias_nao_contam_como_atrasadas_nem_perdidas() {
    let r = relay().await;
    let slow = Relay { target: RelayTarget { udp: proxy(&r.target.udp, Duration::from_millis(150), None).await, ws_url: None }, public: r.public.clone() };
    let (ma, mb, room) = sessions();
    let a = start(&slow, ma, Role::Caller, &room, |c| {
        c.p2p_allow_loopback = true;
        c.test_both_paths = true;
    });
    let b = start(&r, mb, Role::Callee, &room, |c| c.p2p_allow_loopback = true);
    tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        go_direct(&a, &b);
        let (sent0, s0) = (a.stats().sent, b.stats());
        let heard = talk(&a, &b, 150);
        std::thread::sleep(Duration::from_millis(400));
        // As estatisticas do jitter buffer atualizam-se a cada frame tocado.
        let mut out = vec![0i16; FRAME_SAMPLES];
        b.pull_playback(&mut out).unwrap();
        let (sent, sb) = (a.stats().sent - sent0, b.stats());
        eprintln!("copias: enviados={sent} B {sb:?}");
        assert_eq!(sb.received - s0.received, sent, "cada pacote conta uma vez");
        assert_eq!(sb.duplicates - s0.duplicates, sent, "todas as copias do relay descartadas");
        assert_eq!(sb.invalid, 0);
        // O jitter buffer so viu as primeiras copias: nada de repetidos nem de atrasados.
        assert_eq!(sb.jitter.received, sb.received, "{sb:?}");
        assert_eq!((sb.jitter.duplicates, sb.jitter.late), (0, 0), "{sb:?}");
        assert_eq!(sb.jitter.lost, 0, "{sb:?}");
        let audible = heard[20..].iter().filter(|e| **e > 1e4).count();
        // (a "voz" de teste tem silencios: ~78% dos frames passam o limiar, como nos outros testes)
        assert!(audible >= 95, "so {audible} de 130 frames com som; {sb:?}");
    })
    .await
    .unwrap();
}

/// (3) BATERIA: caminho em uso, 10 sondas/s; caiu para o relay, 1 sonda/s; quando o direto
/// volta a responder, outra vez 10/s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sondas_abrandam_com_o_direto_em_baixo() {
    let r = relay().await;
    let (ma, mb, room) = sessions();
    let a = start(&r, ma, Role::Caller, &room, |c| c.p2p_allow_loopback = true);
    let b = start(&r, mb, Role::Callee, &room, |c| c.p2p_allow_loopback = true);
    tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        go_direct(&a, &b);
        std::thread::sleep(Duration::from_secs(1));
        let up = probes_in(&a, 2);
        // O caminho cai (B deixa de responder): A fica so no relay.
        b.p2p_disable();
        wait_for(6, "A devia ter ficado so no relay", || !a.stats().direct);
        let down = probes_in(&a, 4);
        // O direto volta a responder.
        b.p2p_enable();
        b.p2p_set_peer(a.p2p_local_candidates().unwrap());
        wait_for(6, "o direto devia ter voltado", || a.stats().direct);
        std::thread::sleep(Duration::from_millis(500));
        let back = probes_in(&a, 2);
        eprintln!("sondas de A: em uso {up}/2 s, em baixo {down}/4 s, de volta {back}/2 s");
        assert!((16..=24).contains(&up), "em uso: {up} em 2 s (deviam ser ~20)");
        assert!((3..=6).contains(&down), "em baixo: {down} em 4 s (deviam ser ~4)");
        assert!((16..=24).contains(&back), "de volta: {back} em 2 s (deviam ser ~20)");
    })
    .await
    .unwrap();
}

/// COMPATIBILIDADE: relay antigo (nao conhece o pedido de endereco 0x22: nao responde).
/// A recolha acaba sem o IP publico; com enderecos locais o direto funciona na mesma, e
/// sem nenhum a chamada fica no relay como hoje.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_antigo_sem_pedido_de_endereco() {
    let r = relay().await;
    let old = |udp: String| Relay { target: RelayTarget { udp, ws_url: None }, public: r.public.clone() };
    let (ra, rb) = (old(proxy(&r.target.udp, Duration::ZERO, Some(0x22)).await), old(proxy(&r.target.udp, Duration::ZERO, Some(0x22)).await));
    // Com enderecos locais (aqui 127.0.0.1): direto.
    let (ma, mb, room) = sessions();
    let a = start(&ra, ma, Role::Caller, &room, |c| c.p2p_allow_loopback = true);
    let b = start(&rb, mb, Role::Callee, &room, |c| c.p2p_allow_loopback = true);
    // Sem nenhum endereco utilizavel: lista vazia, fica tudo pelo relay.
    let (mc, md, room2) = sessions();
    let c = start(&ra, mc, Role::Caller, &room2, |_| {});
    let d = start(&rb, md, Role::Callee, &room2, |_| {});
    tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        go_direct(&a, &b);
        let heard = talk(&a, &b, 60);
        assert!(heard[20..].iter().filter(|e| **e > 1e4).count() >= 30, "{:?}", b.stats());
        assert!(b.stats().received_direct > 0, "{:?}", b.stats());

        wait_joined(&c);
        wait_joined(&d);
        c.p2p_enable();
        d.p2p_enable();
        wait_for(5, "recolha de C", || c.p2p_local_candidates().is_some());
        assert_eq!(c.p2p_local_candidates(), Some(vec![]), "sem IP publico nem enderecos locais");
        let heard = talk(&c, &d, 60);
        assert!(heard[20..].iter().filter(|e| **e > 1e4).count() >= 30, "{:?}", d.stats());
        assert!(!c.stats().direct && d.stats().received_direct == 0);
    })
    .await
    .unwrap();
}

/// O direto falha SO num sentido (A -> B): B continua a enviar direto, por isso A continua a
/// receber audio pelo direto. Esse audio nao pode fazer A achar que o caminho esta bom: sao
/// as sondas sem resposta que fazem A duplicar pelo relay (~250 ms), e o corte em B e curto.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direto_so_num_sentido_tambem_volta_ao_relay() {
    let r = relay().await;
    let (ma, mb, room) = sessions();
    let a = start(&r, ma, Role::Caller, &room, |c| c.p2p_allow_loopback = true);
    let b = start(&r, mb, Role::Callee, &room, |c| c.p2p_allow_loopback = true);
    tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        go_direct(&a, &b);
        let _ = talk(&a, &b, 60);
        let s0 = b.stats();
        b.test_drop_direct_rx(true);
        // B continua a falar pelo direto enquanto A fala: A recebe audio direto o tempo todo.
        let _ = talk_both(&a, &b, 100);
        let s1 = b.stats();
        let concealed = (s1.plc_frames - s0.plc_frames) + (s1.dred_frames - s0.dred_frames);
        let gave_up = s1.jitter.gave_up - s0.jitter.gave_up;
        eprintln!(
            "um sentido: recebidos {} de 100, inventados {concealed}, desistencias {gave_up}, A {:?}",
            s1.received - s0.received,
            a.stats()
        );
        assert!(a.stats().received_direct > 0, "A devia continuar a receber direto");
        assert!(concealed + gave_up <= CUT_ONE_WAY_MAX, "corte de {} frames em B: {s1:?}", concealed + gave_up);
    })
    .await
    .unwrap();
}
