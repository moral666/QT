//! Video de ponta a ponta (fase 5a): dois motores completos e o relay das chamadas real em
//! localhost. Desde a 0.9.53 o video so vai pela ligacao direta: os testes ligam o direto
//! (`go_direct`) ou poem um proxy no caminho direto (`go_direct_via`) para o estragar, e
//! confirmam que nenhum pacote de video chega ao relay. O "codificador" e sintetico (imagens do
//! tamanho que o debito pede, com o momento de envio la dentro, para medir o atraso); o audio
//! corre nos dois sentidos ao mesmo tempo, como numa chamada. Tempo real: uns segundos por teste.
//!
//! Correr com `--nocapture` para ver os numeros medidos.

use qt_call::codec::FRAME_SAMPLES;
use qt_call::engine::{join_packet, CallEngine, EngineConfig, Net, Pause, RelayTarget};
use qt_core::call::{CallKeys, MediaSession, Role, RoomKeys};
use qt_server::call_relay::{serve, CallLimits, CallStats};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ------------------------------------------------------------------ montagem

struct Relay {
    target: RelayTarget,
    public: Vec<u8>,
    stats: Arc<CallStats>,
}

impl Relay {
    /// Pacotes de video que chegaram ao relay (so as apps antigas os enviam).
    fn video_seen(&self) -> u64 {
        self.stats.video_refused.load(Relaxed)
    }
}

async fn relay() -> Relay {
    let keys = qt_transport::generate_static_keypair().unwrap();
    let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = RelayTarget { udp: udp.local_addr().unwrap().to_string(), ws_url: None };
    let stats = serve(udp, None, &keys.private, CallLimits::default());
    Relay { target, public: keys.public, stats }
}

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

fn start(udp: &str, public: &[u8], media: MediaSession, role: Role, room: &RoomKeys) -> CallEngine {
    let mut c = EngineConfig::new(RelayTarget { udp: udp.to_string(), ws_url: None });
    c.p2p_allow_loopback = true;
    CallEngine::start(c, media, join_packet(public, role, room).unwrap()).unwrap()
}

fn wait_for(secs: u64, what: &str, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(secs), "{what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_joined(e: &CallEngine) {
    wait_for(5, "nao entrou na sala", || e.stats().joined);
}

/// Liga a ligacao direta dos dois lados e troca os enderecos (o que o nucleo faz).
fn go_direct(a: &CallEngine, b: &CallEngine) {
    a.p2p_enable();
    b.p2p_enable();
    wait_for(5, "enderecos de A", || a.p2p_local_candidates().is_some_and(|l| !l.is_empty()));
    wait_for(5, "enderecos de B", || b.p2p_local_candidates().is_some_and(|l| !l.is_empty()));
    a.p2p_set_peer(b.p2p_local_candidates().unwrap());
    b.p2p_set_peer(a.p2p_local_candidates().unwrap());
    wait_for(5, "caminho direto", || a.stats().direct && b.stats().direct);
}

/// Ligacao direta com um proxy no meio (o caminho de A para B passa por `bad`): A so conhece a
/// frente do proxy e B so conhece as costas. Chamar dentro de `spawn_blocking`.
fn go_direct_via(a: &CallEngine, b: &CallEngine, bad: Arc<Bad>) {
    a.p2p_enable();
    b.p2p_enable();
    wait_for(5, "enderecos de A", || a.p2p_local_candidates().is_some_and(|l| !l.is_empty()));
    wait_for(5, "enderecos de B", || b.p2p_local_candidates().is_some_and(|l| !l.is_empty()));
    let cands = b.p2p_local_candidates().unwrap();
    let b_addr = cands.iter().find(|x| x.ip().is_loopback()).or(cands.first()).copied().unwrap();
    let (front, back) = tokio::runtime::Handle::current().block_on(proxy(&b_addr.to_string(), bad));
    a.p2p_set_peer(vec![front.parse().unwrap()]);
    b.p2p_set_peer(vec![back.parse().unwrap()]);
    wait_for(5, "caminho direto pelo proxy", || a.stats().direct && b.stats().direct);
}

/// Proxy UDP no caminho direto (entre A e o endereco de B), com o que for preciso para estragar
/// o sentido de A para B. Devolve (frente, para A; costas, para B).
#[derive(Default)]
struct Bad {
    /// Atraso de A para B (ms).
    delay_ms: AtomicU32,
    /// % dos pacotes de VIDEO deitados fora (de A para B).
    video_loss_pct: AtomicU32,
    /// Deita fora todo o video (nos dois sentidos): caminho que nao deixa passar video.
    block_video: AtomicBool,
    /// Debito maximo de A para B (bit/s, 0 = sem limite), com 100 ms de fila.
    rate_bps: AtomicU32,
}

async fn proxy(relay: &str, bad: Arc<Bad>) -> (String, String) {
    let front = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let back = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    back.connect(relay).await.unwrap();
    let client = Arc::new(Mutex::new(None::<SocketAddr>));
    let (tx, mut rx) = tokio::sync::mpsc::channel::<(Instant, Vec<u8>)>(4096);
    {
        let (front, client, bad) = (front.clone(), client.clone(), bad.clone());
        tokio::spawn(async move {
            let mut buf = vec![0u8; 2048];
            let mut seed = 0x2545_F491_4F6C_DD1Du64;
            loop {
                let Ok((n, from)) = front.recv_from(&mut buf).await else { continue };
                *client.lock().unwrap() = Some(from);
                let video = n > 0 && buf[0] & !1 == 0x40;
                if video && bad.block_video.load(Relaxed) {
                    continue;
                }
                if video {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    if ((seed % 100) as u32) < bad.video_loss_pct.load(Relaxed) {
                        continue;
                    }
                }
                let at = Instant::now() + Duration::from_millis(bad.delay_ms.load(Relaxed) as u64);
                let _ = tx.try_send((at, buf[..n].to_vec()));
            }
        });
    }
    {
        // Saida para B: atraso e limite de debito (fila de 100 ms, o resto cai).
        let (back, bad) = (back.clone(), bad.clone());
        tokio::spawn(async move {
            let mut free_at = Instant::now();
            while let Some((at, data)) = rx.recv().await {
                if let Some(d) = at.checked_duration_since(Instant::now()) {
                    tokio::time::sleep(d).await;
                }
                let rate = bad.rate_bps.load(Relaxed);
                if rate > 0 {
                    let now = Instant::now();
                    if free_at < now {
                        free_at = now;
                    }
                    if free_at.duration_since(now) > Duration::from_millis(100) {
                        continue; // fila cheia: cai
                    }
                    free_at += Duration::from_secs_f64(data.len() as f64 * 8.0 / rate as f64);
                    if let Some(d) = free_at.checked_duration_since(Instant::now()) {
                        tokio::time::sleep(d).await;
                    }
                }
                let _ = back.send(&data).await;
            }
        });
    }
    let back_addr = back.local_addr().unwrap().to_string();
    {
        let (front, bad) = (front.clone(), bad.clone());
        tokio::spawn(async move {
            let mut buf = vec![0u8; 2048];
            loop {
                let Ok(n) = back.recv(&mut buf).await else { continue };
                if bad.block_video.load(Relaxed) && n > 0 && (buf[0] & !1 == 0x40 || buf[0] == 0x24) {
                    continue;
                }
                let to = *client.lock().unwrap();
                if let Some(c) = to {
                    let _ = front.send_to(&buf[..n], c).await;
                }
            }
        });
    }
    (front.local_addr().unwrap().to_string(), back_addr)
}

// ------------------------------------------------------------------ chamada com video

/// O que chegou num sentido.
#[derive(Default, Debug)]
struct Seen {
    frames: u64,
    keyframes: u64,
    /// Atraso de cada imagem: entregue ao codificador -> pronta a descodificar (ms).
    latency_ms: Vec<f64>,
    /// Maior intervalo entre imagens seguidas depois da primeira (ms).
    max_gap_ms: f64,
    bytes: u64,
}

impl Seen {
    fn pct(&self, p: f64) -> f64 {
        let mut v = self.latency_ms.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if v.is_empty() {
            return f64::NAN;
        }
        v[((v.len() as f64 - 1.0) * p).round() as usize]
    }
}

/// Codificador sintetico: segue os parametros do motor (debito, imagens/s, imagem completa).
fn encoder(e: &CallEngine, t0: Instant, until: Instant) -> u64 {
    let mut last_seq = None;
    let mut n = 0u64;
    let mut next = Instant::now();
    while Instant::now() < until {
        let p = e.video_params();
        let fps = p.fps.max(1) as u64;
        if p.send {
            let key = last_seq != Some(p.keyframe_seq);
            let size = ((p.bitrate_bps as u64 / 8 / fps) as usize * if key { 4 } else { 1 }).max(64);
            let mut data = vec![0x5Au8; size];
            data[..8].copy_from_slice(&(t0.elapsed().as_nanos() as u64).to_le_bytes());
            data[8..16].copy_from_slice(&n.to_le_bytes());
            if e.video_push(&data, key, 1, 0) {
                if key {
                    last_seq = Some(p.keyframe_seq);
                }
                n += 1;
            }
        }
        next += Duration::from_millis(1000 / fps);
        if let Some(d) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(d);
        } else {
            next = Instant::now();
        }
    }
    n
}

fn receiver(e: &CallEngine, t0: Instant, until: Instant, seen: &Mutex<Seen>) {
    let mut last: Option<Instant> = None;
    while Instant::now() < until {
        if let Some(f) = e.video_pull(Duration::from_millis(50)) {
            let now = Instant::now();
            let sent = u64::from_le_bytes(f.data[..8].try_into().unwrap());
            assert_eq!(f.rotation, 1, "a rotacao chega");
            let mut s = seen.lock().unwrap();
            s.frames += 1;
            s.keyframes += f.keyframe as u64;
            s.bytes += f.data.len() as u64;
            s.latency_ms.push((t0.elapsed().as_nanos() as u64).saturating_sub(sent) as f64 / 1e6);
            if let Some(l) = last {
                s.max_gap_ms = s.max_gap_ms.max(now.duration_since(l).as_secs_f64() * 1000.0);
            }
            last = Some(now);
        }
    }
}

fn voice(i: usize) -> Vec<i16> {
    (0..FRAME_SAMPLES).map(|k| (((i * FRAME_SAMPLES + k) as f64 * 0.04).sin() * 6000.0) as i16).collect()
}

/// Uma chamada com video durante `secs` segundos. `a_video`/`b_video`: quem envia video
/// (o outro aceita receber). `each_second(s)` corre a cada segundo (para estragar a rede a
/// meio). Devolve (o que B viu de A, o que A viu de B).
fn video_call(a: &CallEngine, b: &CallEngine, secs: u64, a_video: bool, b_video: bool, mut each_second: impl FnMut(u64)) -> (Seen, Seen) {
    a.video_set_sending(a_video);
    b.video_set_receiving(a_video);
    b.video_set_sending(b_video);
    a.video_set_receiving(b_video);
    let t0 = Instant::now();
    let until = t0 + Duration::from_secs(secs);
    let (seen_b, seen_a) = (Mutex::new(Seen::default()), Mutex::new(Seen::default()));
    std::thread::scope(|sc| {
        if a_video {
            sc.spawn(|| encoder(a, t0, until));
            sc.spawn(|| receiver(b, t0, until, &seen_b));
        }
        if b_video {
            sc.spawn(|| encoder(b, t0, until));
            sc.spawn(|| receiver(a, t0, until, &seen_a));
        }
        // Audio nos dois sentidos, ao ritmo real.
        let mut out = vec![0i16; FRAME_SAMPLES];
        let mut i = 0usize;
        let mut sec = 0;
        while Instant::now() < until {
            a.push_capture(&voice(i)).unwrap();
            b.push_capture(&voice(i + 3)).unwrap();
            a.pull_playback(&mut out).unwrap();
            b.pull_playback(&mut out).unwrap();
            i += 1;
            if i.is_multiple_of(50) {
                sec += 1;
                each_second(sec);
            }
            let next = t0 + Duration::from_millis(20 * i as u64);
            if let Some(d) = next.checked_duration_since(Instant::now()) {
                std::thread::sleep(d);
            }
        }
    });
    (seen_b.into_inner().unwrap(), seen_a.into_inner().unwrap())
}

fn kbps(bytes: u64, secs: f64) -> f64 {
    bytes as f64 * 8.0 / secs / 1000.0
}

/// Frames de audio inventados (PLC/DRED) ou abandonados em `e` desde `s0`.
fn audio_concealed(e: &CallEngine, s0: &qt_call::engine::EngineStats) -> u64 {
    let s = e.stats();
    (s.plc_frames - s0.plc_frames) + (s.dred_frames - s0.dred_frames) + (s.jitter.gave_up - s0.jitter.gave_up)
}

// ------------------------------------------------------------------ testes

/// Sem ligacao direta: so voz, o video fica em pausa e nada de video sai (nem para o relay).
/// Com o direto: video ate 1080p (Wi-Fi), sem perdas falsas, e o relay continua sem video.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn video_so_pelo_direto_e_nunca_pelo_relay() {
    let r = relay().await;
    let (ma, mb, room) = sessions();
    let a = start(&r.target.udp, &r.public, ma, Role::Caller, &room);
    let b = start(&r.target.udp, &r.public, mb, Role::Callee, &room);
    tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        a.set_network(Net::Wifi);
        b.set_network(Net::Wifi);
        let s0b = b.stats();
        let (seen, _) = video_call(&a, &b, 4, true, false, |_| {});
        let p = a.video_params();
        assert_eq!((p.send, p.paused), (false, Some(Pause::NoDirect)), "{p:?}");
        assert_eq!(seen.frames, 0);
        assert_eq!((a.video_stats().sent_packets, b.video_stats().sent_packets), (0, 0), "nada de video sai sem direto");
        assert_eq!(r.video_seen(), 0, "nenhum pacote de video no relay");
        assert!(audio_concealed(&b, &s0b) < 10, "a voz passa pelo relay");

        go_direct(&a, &b);
        let mut log = Vec::new();
        let (seen, _) = video_call(&a, &b, 12, true, false, |s| log.push((s, a.video_params())));
        let (va, vb) = (a.video_stats(), b.video_stats());
        let p = a.video_params();
        eprintln!(
            "DIRETO: {} imagens ({} completas), atraso mediana {:.0} ms p95 {:.0} ms, maior paragem {:.0} ms; \
             debito pedido {} kbit/s em {}p; na rede {:.0} kbit/s; perda medida {}/{}; A/V {} ms",
            seen.frames, seen.keyframes, seen.pct(0.5), seen.pct(0.95), seen.max_gap_ms,
            p.bitrate_bps / 1000, p.height, kbps(va.sent_bytes, 12.0), vb.recv.reported, vb.recv.expected, vb.recv.av_lag_ms
        );
        for (s, p) in log.iter().step_by(2) {
            eprintln!("  {s:>2} s: {}p {} kbit/s", p.height, p.bitrate_bps / 1000);
        }
        assert!(p.send && p.paused.is_none() && p.height >= 720, "{p:?}");
        assert!(seen.frames >= 300, "{seen:?}");
        assert!(seen.pct(0.95) < 250.0, "atraso: {seen:?}");
        assert_eq!(vb.recv.lost_frames, 0, "{vb:?}");
        assert_eq!(vb.recv.reported, vb.recv.expected, "sem perdas falsas: {vb:?}");
        assert_eq!(r.video_seen(), 0, "nenhum pacote de video no relay, nem com direto");
    })
    .await
    .unwrap();
}

/// Video nos dois sentidos (o caso normal): os relatorios de cada lado partilham o contador
/// com as imagens. A perda medida tem de ser zero. O caminho direto de A para B tem 150 ms.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn video_nos_dois_sentidos_sem_perdas_falsas() {
    let r = relay().await;
    let bad = Arc::new(Bad::default());
    bad.delay_ms.store(150, Relaxed);
    let (ma, mb, room) = sessions();
    let a = start(&r.target.udp, &r.public, ma, Role::Caller, &room);
    let b = start(&r.target.udp, &r.public, mb, Role::Callee, &room);
    tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        go_direct_via(&a, &b, bad.clone());
        let (s0a, s0b) = (a.stats(), b.stats());
        let (ab, ba) = video_call(&a, &b, 12, true, true, |_| {});
        let (va, vb) = (a.video_stats(), b.video_stats());
        eprintln!(
            "DOIS SENTIDOS (+150 ms): A->B {} imagens, atraso {:.0}/{:.0} ms; B->A {} imagens, atraso {:.0}/{:.0} ms; \
             A {} kbit/s, B {} kbit/s; perdas medidas A {}/{} B {}/{}; paragens {} {}",
            ab.frames, ab.pct(0.5), ab.pct(0.95), ba.frames, ba.pct(0.5), ba.pct(0.95),
            a.video_params().bitrate_bps / 1000, b.video_params().bitrate_bps / 1000,
            va.recv.reported, va.recv.expected, vb.recv.reported, vb.recv.expected, va.stalls, vb.stalls
        );
        for (who, v) in [("A", &va), ("B", &vb)] {
            assert_eq!(v.recv.reported, v.recv.expected, "{who}: perdas falsas: {v:?}");
            assert_eq!((v.recv.lost_frames, v.stalls), (0, 0), "{who}: {v:?}");
        }
        assert!(a.video_params().bitrate_bps >= 2_000_000, "nada travou a subida de A: {:?}", a.video_params());
        assert!(b.video_params().bitrate_bps >= 2_000_000, "nada travou a subida de B: {:?}", b.video_params());
        assert!(ab.frames >= 300 && ba.frames >= 300);
        assert!(audio_concealed(&b, &s0b) < 15 && audio_concealed(&a, &s0a) < 15, "o audio sofreu");
        assert_eq!(r.video_seen(), 0);
    })
    .await
    .unwrap();
}

/// Perda VERDADEIRA de 10% dos pacotes de video no caminho direto de A: tem de ser medida (~10%)
/// e o debito tem de baixar. Prova que as correcoes das perdas falsas nao escondem as reais.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn perda_real_de_10_por_cento_e_medida_e_baixa_o_debito() {
    let r = relay().await;
    let bad = Arc::new(Bad::default());
    let (ma, mb, room) = sessions();
    let a = start(&r.target.udp, &r.public, ma, Role::Caller, &room);
    let b = start(&r.target.udp, &r.public, mb, Role::Callee, &room);
    tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        // Wi-Fi dos dois lados: arranca em 1,2 Mbit/s (com dados moveis arrancava em 500 kbit/s).
        a.set_network(Net::Wifi);
        b.set_network(Net::Wifi);
        go_direct_via(&a, &b, bad.clone());
        // 6 s limpos (sobe), depois 10% de perda.
        let mut at_loss = (0, 0, 0u64, 0u64);
        let mut log = Vec::new();
        let s0b = b.stats();
        let (seen, _) = video_call(&a, &b, 16, true, false, |s| {
            if s == 6 {
                let v = b.video_stats();
                at_loss = (a.video_params().bitrate_bps, 0, v.recv.reported, v.recv.expected);
                bad.video_loss_pct.store(10, Relaxed);
            }
            log.push((s, a.video_params(), a.video_stats().peer_loss_permille));
        });
        let vb = b.video_stats();
        let (got, exp) = (vb.recv.reported - at_loss.2, vb.recv.expected - at_loss.3);
        let loss = 1.0 - got as f64 / exp as f64;
        let p = a.video_params();
        eprintln!(
            "PERDA REAL 10%: medida {:.1}% ({got}/{exp}); debito {} -> {} kbit/s ({}p, pausa {:?}); \
             imagens perdidas {}, pedidos de imagem completa {}; {} imagens vistas",
            loss * 100.0, at_loss.0 / 1000, p.bitrate_bps / 1000, p.height, p.paused,
            vb.recv.lost_frames, vb.recv.keyframe_requests, seen.frames
        );
        for (s, p, l) in &log {
            eprintln!("  {s:>2} s: {}p {} kbit/s, perda no relatorio {:.1}%", p.height, p.bitrate_bps / 1000, *l as f64 / 10.0);
        }
        assert!((0.07..=0.13).contains(&loss), "perda medida {loss:.3}");
        assert!(at_loss.0 >= 2_000_000, "antes da perda tinha subido: {}", at_loss.0);
        assert!(p.bitrate_bps <= at_loss.0 / 2 || p.paused.is_some(), "o debito tinha de baixar: {p:?}");
        assert!(vb.recv.lost_frames > 0 && vb.recv.keyframe_requests > 0);
        // A perda era so no video: o audio nao sofreu.
        assert!(audio_concealed(&b, &s0b) < 15, "o audio sofreu");
        assert_eq!(r.video_seen(), 0);
    })
    .await
    .unwrap();
}

/// O direto cai a meio: o video pausa (nunca passa para o relay) e a voz continua; quando o
/// direto volta, o video volta sozinho.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn direto_que_cai_pausa_o_video_e_volta() {
    let r = relay().await;
    let (ma, mb, room) = sessions();
    let a = start(&r.target.udp, &r.public, ma, Role::Caller, &room);
    let b = start(&r.target.udp, &r.public, mb, Role::Callee, &room);
    tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        a.set_network(Net::Wifi);
        b.set_network(Net::Wifi);
        go_direct(&a, &b);
        let mut log = Vec::new();
        let mut before = (0u32, 0u16);
        let (mut frames_at_cut, mut frames_cut_end, mut paused_mid) = (0u64, 0u64, None);
        let (seen, _) = video_call(&a, &b, 32, true, false, |s| {
            match s {
                12 => {
                    before = (a.video_params().bitrate_bps, a.video_params().height);
                    b.test_drop_direct_rx(true); // o direto morre
                }
                17 => {
                    frames_at_cut = b.video_stats().recv.frames;
                    paused_mid = Some(a.video_params());
                }
                21 => {
                    frames_cut_end = b.video_stats().recv.frames;
                    b.test_drop_direct_rx(false); // e volta
                }
                _ => {}
            }
            log.push((s, a.video_params(), a.stats().direct));
        });
        let vb = b.video_stats();
        let p = a.video_params();
        eprintln!(
            "DIRETO QUE CAI: antes {}p {} kbit/s; no corte {:?}; depois {}p {} kbit/s; imagens no corte {}; total {}",
            before.1, before.0 / 1000, paused_mid.map(|p| p.paused), p.height, p.bitrate_bps / 1000,
            frames_cut_end - frames_at_cut, seen.frames
        );
        for (s, p, d) in log.iter().step_by(2) {
            eprintln!("  {s:>2} s: {}p {} kbit/s{}{}", p.height, p.bitrate_bps / 1000, if *d { " (direto)" } else { "" }, if p.paused.is_some() { " (pausa)" } else { "" });
        }
        assert!(before.1 >= 720 && before.0 >= 2_200_000, "o direto devia ter subido: {before:?}");
        let mid = paused_mid.unwrap();
        assert_eq!((mid.send, mid.paused), (false, Some(Pause::NoDirect)), "sem direto o video pausa: {mid:?}");
        assert_eq!(frames_cut_end, frames_at_cut, "sem direto nao chega video");
        assert!(p.send && p.paused.is_none(), "o video voltou com o direto: {p:?}");
        assert!(vb.recv.frames > frames_cut_end + 150, "chegaram imagens depois de o direto voltar: {vb:?}");
        assert_eq!(r.video_seen(), 0, "nunca passou para o relay");
    })
    .await
    .unwrap();
}

/// Caminho direto que deixa de passar video durante 8 s: pausa (a voz continua) e volta
/// sozinho, sem perdas falsas depois de retomar.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn pausa_e_volta_sem_perdas_falsas() {
    let r = relay().await;
    let bad = Arc::new(Bad::default());
    let (ma, mb, room) = sessions();
    let a = start(&r.target.udp, &r.public, ma, Role::Caller, &room);
    let b = start(&r.target.udp, &r.public, mb, Role::Callee, &room);
    tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        go_direct_via(&a, &b, bad.clone());
        let mut paused_at = None;
        let mut resumed_at = None;
        // Pacotes que B tinha recebido quando A voltou, e se ja lhe chegaram pacotes novos.
        let mut packets_at_resume = None;
        let mut new_packets_seen = false;
        let mut stalls_at_resume = None;
        let mut after_resume = None;
        let s0b = b.stats();
        video_call(&a, &b, 26, true, false, |s| {
            match s {
                4 => bad.video_loss_pct.store(100, Relaxed),
                12 => bad.video_loss_pct.store(0, Relaxed),
                _ => {}
            }
            let p = a.video_params();
            if p.paused == Some(Pause::WeakNetwork) && paused_at.is_none() {
                paused_at = Some(s);
            }
            if paused_at.is_some() && p.send && resumed_at.is_none() {
                resumed_at = Some(s);
                packets_at_resume = Some(b.video_stats().recv.packets);
                stalls_at_resume = Some(a.video_stats().stalls);
            } else if let (Some(n), None) = (packets_at_resume, after_resume) {
                // Os pacotes perdidos durante o corte (reais, de ANTES de voltar) so entram nas
                // contas de B no primeiro relatorio (250 ms) depois de lhe chegar um pacote novo:
                // ate la B nao sabe que faltam. A fotografia e tirada no segundo a seguir a esse
                // pacote, para essas perdas nao contarem como perdas depois de voltar.
                if new_packets_seen {
                    let v = b.video_stats();
                    after_resume = Some((v.recv.reported, v.recv.expected));
                } else if b.video_stats().recv.packets > n {
                    new_packets_seen = true;
                }
            }
        });
        let (va, vb) = (a.video_stats(), b.video_stats());
        let (p0, e0) = after_resume.expect("devia ter voltado e o video chegado a B");
        let st0 = stalls_at_resume.expect("devia ter voltado");
        eprintln!(
            "PAUSA: pausou aos {paused_at:?} s, voltou aos {resumed_at:?} s; depois de voltar {}/{} pacotes, \
             paragens {} -> {}; debito final {} kbit/s",
            vb.recv.reported - p0, vb.recv.expected - e0, st0, va.stalls, a.video_params().bitrate_bps / 1000
        );
        assert!(paused_at.is_some_and(|s| s <= 12), "devia ter pausado");
        assert!(resumed_at.is_some_and(|s| s <= 22), "devia ter voltado");
        assert_eq!(vb.recv.reported - p0, vb.recv.expected - e0, "perdas falsas depois de voltar");
        assert_eq!(va.stalls, st0, "'nada chega' falso depois de voltar");
        assert!(a.video_params().send && a.video_params().bitrate_bps >= 200_000);
        assert!(audio_concealed(&b, &s0b) < 15, "a voz continuou");
        assert_eq!(r.video_seen(), 0);
    })
    .await
    .unwrap();
}

/// Caminho direto que nunca deixa passar video (rede que o corta): o video nunca chega, pausa
/// ao fim de ~4 s e a voz continua, sem erro.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn direto_sem_video_pausa_e_a_voz_continua() {
    let r = relay().await;
    let bad = Arc::new(Bad::default());
    bad.block_video.store(true, Relaxed);
    let (ma, mb, room) = sessions();
    let a = start(&r.target.udp, &r.public, ma, Role::Caller, &room);
    let b = start(&r.target.udp, &r.public, mb, Role::Callee, &room);
    tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        go_direct_via(&a, &b, bad.clone());
        let s0b = b.stats();
        let (seen, _) = video_call(&a, &b, 7, true, false, |_| {});
        let p = a.video_params();
        assert_eq!(p.paused, Some(Pause::WeakNetwork), "{p:?}");
        assert_eq!(seen.frames, 0);
        assert!(audio_concealed(&b, &s0b) < 10, "a voz passa");
        assert_eq!(r.video_seen(), 0);
    })
    .await
    .unwrap();
}

/// Consentimento: sem o nucleo autorizar, nada sai; sem quem recebe aceitar, nada e lido.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn nada_sai_nem_e_lido_sem_autorizacao() {
    let r = relay().await;
    let (ma, mb, room) = sessions();
    let a = start(&r.target.udp, &r.public, ma, Role::Caller, &room);
    let b = start(&r.target.udp, &r.public, mb, Role::Callee, &room);
    tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        go_direct(&a, &b);
        // Camara de A ligada, mas B ainda nao aceitou: o nucleo nao autoriza o envio.
        assert!(!a.video_push(&[1u8; 3000], true, 0, 0));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(a.video_stats().sent_packets, 0);
        // Autorizado em A mas B nao aceitou receber (app alterada do lado de A): B nao le nada.
        a.video_set_sending(true);
        assert!(a.video_push(&[1u8; 3000], true, 0, 0));
        std::thread::sleep(Duration::from_millis(500));
        assert!(a.video_stats().sent_packets > 0);
        assert_eq!(b.video_stats().recv.packets, 0);
        assert!(b.video_pull(Duration::from_millis(100)).is_none());
        assert_eq!(r.video_seen(), 0);
    })
    .await
    .unwrap();
}

/// O audio tem prioridade: com o caminho direto de A limitado a 700 kbit/s, o video desce ate
/// caber e o audio fica limpo.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn rede_estreita_baixa_o_video_e_o_audio_fica_limpo() {
    let r = relay().await;
    let bad = Arc::new(Bad::default());
    bad.rate_bps.store(700_000, Relaxed);
    let (ma, mb, room) = sessions();
    let a = start(&r.target.udp, &r.public, ma, Role::Caller, &room);
    let b = start(&r.target.udp, &r.public, mb, Role::Callee, &room);
    tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        go_direct_via(&a, &b, bad.clone());
        let mut s_mid = None;
        let mut log = Vec::new();
        let (seen, _) = video_call(&a, &b, 20, true, false, |s| {
            if s == 10 {
                s_mid = Some(b.stats());
            }
            log.push((s, a.video_params()));
        });
        let s_mid = s_mid.unwrap();
        let concealed = audio_concealed(&b, &s_mid);
        let p = a.video_params();
        eprintln!(
            "REDE ESTREITA (700 kbit/s): video {}p {} kbit/s; audio inventado nos ultimos 10 s: {concealed} de 500 frames; \
             {} imagens, maior paragem {:.0} ms",
            p.height, p.bitrate_bps / 1000, seen.frames, seen.max_gap_ms
        );
        for (s, p) in log.iter().step_by(2) {
            eprintln!("  {s:>2} s: {}p {} kbit/s", p.height, p.bitrate_bps / 1000);
        }
        assert!(p.bitrate_bps < 700_000, "o video tinha de caber: {p:?}");
        assert!(concealed <= 10, "o audio sofreu: {concealed} frames em 10 s");
        assert_eq!(r.video_seen(), 0);
    })
    .await
    .unwrap();
}

/// Wi-Fi dos dois lados e aparelhos que fazem 4K (0.9.61): sem teto fixo, sobe acima dos 1080p
/// (o descodificador de B diz o que aguenta; a capacidade passa pelo direto, nunca pelo relay).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wifi_com_aparelhos_4k_sobe_acima_de_1080p() {
    let r = relay().await;
    let (ma, mb, room) = sessions();
    let a = start(&r.target.udp, &r.public, ma, Role::Caller, &room);
    let b = start(&r.target.udp, &r.public, mb, Role::Callee, &room);
    tokio::task::spawn_blocking(move || {
        wait_joined(&a);
        wait_joined(&b);
        a.set_network(Net::Wifi);
        b.set_network(Net::Wifi);
        a.video_set_limits(Some(2160), None);
        b.video_set_limits(None, Some(2160));
        go_direct(&a, &b);
        let mut log = Vec::new();
        let (seen, _) = video_call(&a, &b, 18, true, false, |s| log.push((s, a.video_params())));
        let p = a.video_params();
        eprintln!("WI-FI 4K: {}p {} kbit/s, {} imagens", p.height, p.bitrate_bps / 1000, seen.frames);
        for (s, p) in log.iter().step_by(2) {
            eprintln!("  {s:>2} s: {}p {} kbit/s", p.height, p.bitrate_bps / 1000);
        }
        assert!(p.send && p.height >= 1440, "{p:?}");
        // A 16 Mbit/s em localhost, com os outros testes a correr, perde-se uma ou outra imagem
        // (o controlo de debito ve e desce): no maximo 1 %.
        let lost = b.video_stats().recv.lost_frames;
        assert!(lost * 100 <= seen.frames.max(1), "{lost} imagens perdidas de {}", seen.frames);
        assert_eq!(r.video_seen(), 0, "nada de video no relay");
    })
    .await
    .unwrap();
}
