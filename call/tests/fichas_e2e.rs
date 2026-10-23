//! Fichas do video pelo relay (0006) com os motores completos e o relay das chamadas real em
//! localhost (registo das gastas no Redis em 127.0.0.1:6379):
//!   * quem tem fichas envia video pelo relay e gasta uma por bloco;
//!   * quem nao tem: o video pausa pelo relay (`Pause::RelayLimit`) e a voz continua;
//!   * em ligacao direta nao se gasta nada e o video continua;
//!   * uma ficha ja gasta (a app morreu a meio) passa a seguinte;
//!   * relay sem fichas, ou antigo (nao responde a chave), video como antes;
//!   * a chave da sala tem de ser a publicada no site: outra, ou nenhuma (site indisponivel),
//!     nada se gasta e o video pelo relay fica em pausa (voz e direto normais).

use qt_call::codec::FRAME_SAMPLES;
use qt_call::engine::{join_packet, CallEngine, EngineConfig, Pause, RelayTarget};
use qt_call::fichas::{FichasConfig, MemWallet};
use qt_core::call::{CallKeys, MediaSession, Role, RoomKeys};
use qt_core::fichas::{self, Ficha, IssueRequest, Issuer};
use qt_server::call_relay::{serve, CallLimits, FichasSetup};
use qt_server::fichas::SpentStore;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SEED: [u8; 32] = [0x77; 32];

struct Relay {
    udp: String,
    public: Vec<u8>,
}

static N: AtomicU64 = AtomicU64::new(0);

/// Relay das chamadas com fichas (semente num ficheiro, gastas no Redis com um prefixo so deste teste).
async fn relay(fichas_on: bool) -> (Relay, Arc<SpentStore>) {
    let keys = qt_transport::generate_static_keypair().unwrap();
    let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = udp.local_addr().unwrap();
    let n = N.fetch_add(1, Relaxed);
    let dir = std::env::temp_dir().join(format!("qt-call-fichas-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let seed = dir.join("fichas_seed.bin");
    std::fs::write(&seed, SEED).unwrap();
    let spent = Arc::new(SpentStore::with_prefix("redis://127.0.0.1:6379", &format!("fichas-c-{}-{n}", std::process::id())).unwrap());
    let setup = FichasSetup { seed_path: Some(seed), spent: Some(spent.clone()) };
    serve(udp, None, &keys.private, CallLimits { fichas: fichas_on, ..Default::default() }, setup);
    (Relay { udp: addr.to_string(), public: keys.public }, spent)
}

fn wallet(n: usize) -> Arc<MemWallet> {
    MemWallet::new(fichas_n(n))
}

fn fichas_n(n: usize) -> Vec<Ficha> {
    let epoch = fichas::current_epoch();
    let iss = Issuer::derive(&SEED, epoch).unwrap();
    let (req, blinded) = IssueRequest::new(epoch, n).unwrap();
    let (ev, proof) = iss.issue(&blinded).unwrap();
    req.finalize(&iss.public_key(), &ev, &proof).unwrap()
}

fn sessions() -> (MediaSession, MediaSession, RoomKeys) {
    let rnd = |k: u8| -> [u8; 32] {
        let mut b = [k; 32];
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

/// Chave publicada no site para este mes (a do relay de teste).
fn site() -> Vec<(u32, [u8; 32])> {
    let e = fichas::current_epoch();
    vec![(e, Issuer::derive(&SEED, e).unwrap().public_key())]
}

fn start(udp: &str, public: &[u8], media: MediaSession, role: Role, room: &RoomKeys, w: Option<Arc<MemWallet>>) -> CallEngine {
    start_with(udp, public, media, role, room, w, site())
}

fn start_with(
    udp: &str,
    public: &[u8],
    media: MediaSession,
    role: Role,
    room: &RoomKeys,
    w: Option<Arc<MemWallet>>,
    published: Vec<(u32, [u8; 32])>,
) -> CallEngine {
    let mut c = EngineConfig::new(RelayTarget { udp: udp.to_string(), ws_url: None });
    c.p2p_allow_loopback = true;
    c.fichas = w.map(|w| FichasConfig { wallet: w, relay_public_key: public.to_vec(), published });
    CallEngine::start(c, media, join_packet(public, role, room).unwrap()).unwrap()
}

fn wait_for(secs: u64, what: &str, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(secs), "{what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn go_direct(a: &CallEngine, b: &CallEngine) {
    a.p2p_enable();
    b.p2p_enable();
    wait_for(5, "enderecos de A", || a.p2p_local_candidates().is_some_and(|l| !l.is_empty()));
    wait_for(5, "enderecos de B", || b.p2p_local_candidates().is_some_and(|l| !l.is_empty()));
    a.p2p_set_peer(b.p2p_local_candidates().unwrap());
    b.p2p_set_peer(a.p2p_local_candidates().unwrap());
    wait_for(5, "caminho direto", || a.stats().direct && b.stats().direct);
}

fn voice(i: usize) -> Vec<i16> {
    (0..FRAME_SAMPLES).map(|k| (((i * FRAME_SAMPLES + k) as f64 * 0.04).sin() * 6000.0) as i16).collect()
}

fn encoder(e: &CallEngine, until: Instant) {
    let mut last_seq = None;
    let mut next = Instant::now();
    while Instant::now() < until {
        let p = e.video_params();
        let fps = p.fps.max(1) as u64;
        if p.send {
            let key = last_seq != Some(p.keyframe_seq);
            let size = ((p.bitrate_bps as u64 / 8 / fps) as usize * if key { 4 } else { 1 }).max(64);
            if e.video_push(&vec![0x5Au8; size], key, 0, 0) && key {
                last_seq = Some(p.keyframe_seq);
            }
        }
        next += Duration::from_millis(1000 / fps);
        match next.checked_duration_since(Instant::now()) {
            Some(d) => std::thread::sleep(d),
            None => next = Instant::now(),
        }
    }
}

fn receiver(e: &CallEngine, until: Instant, frames: &AtomicU64) {
    while Instant::now() < until {
        if e.video_pull(Duration::from_millis(50)).is_some() {
            frames.fetch_add(1, Relaxed);
        }
    }
}

/// Chamada com video dos dois lados (cada um aceita o do outro) durante `secs`. Devolve
/// (imagens de A vistas por B, imagens de B vistas por A, audio recebido por A, por B).
fn video_call(a: &CallEngine, b: &CallEngine, secs: u64) -> (u64, u64, u64, u64) {
    for e in [a, b] {
        e.video_set_sending(true);
        e.video_set_receiving(true);
    }
    let (ra0, rb0) = (a.stats().received, b.stats().received);
    let t0 = Instant::now();
    let until = t0 + Duration::from_secs(secs);
    let (seen_b, seen_a) = (AtomicU64::new(0), AtomicU64::new(0));
    std::thread::scope(|sc| {
        sc.spawn(|| encoder(a, until));
        sc.spawn(|| encoder(b, until));
        sc.spawn(|| receiver(b, until, &seen_b));
        sc.spawn(|| receiver(a, until, &seen_a));
        let mut out = vec![0i16; FRAME_SAMPLES];
        let mut i = 0usize;
        while Instant::now() < until {
            a.push_capture(&voice(i)).unwrap();
            b.push_capture(&voice(i + 3)).unwrap();
            a.pull_playback(&mut out).unwrap();
            b.pull_playback(&mut out).unwrap();
            i += 1;
            if let Some(d) = (t0 + Duration::from_millis(20 * i as u64)).checked_duration_since(Instant::now()) {
                std::thread::sleep(d);
            }
        }
    });
    (seen_b.into_inner(), seen_a.into_inner(), a.stats().received - ra0, b.stats().received - rb0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn com_fichas_video_pelo_relay_sem_fichas_so_voz() {
    let (r, _) = relay(true).await;
    let (ma, mb, room) = sessions();
    let wa = wallet(3);
    let a = start(&r.udp, &r.public, ma, Role::Caller, &room, Some(wa.clone()));
    // B e uma app sem fichas (antiga, ou sem fichas este mes).
    let b = start(&r.udp, &r.public, mb, Role::Callee, &room, Some(MemWallet::new(Vec::new())));
    tokio::task::spawn_blocking(move || {
        wait_for(5, "na sala", || a.stats().joined && b.stats().joined);
        let (a_to_b, b_to_a, audio_a, audio_b) = video_call(&a, &b, 8);
        let (va, vb) = (a.video_stats(), b.video_stats());
        eprintln!("FICHAS: A->B {a_to_b} imagens (fichas gastas {}, credito {} s); B->A {b_to_a}; voz {audio_a}/{audio_b}", va.fichas_spent, va.relay_credit_secs);
        assert!(a_to_b >= 100, "A tem fichas: o video passa pelo relay ({a_to_b})");
        assert_eq!((va.fichas_spent, wa.left()), (1, 2), "um bloco de 1 minuto");
        assert!(va.relay_credit_secs > 45);
        assert_eq!(wa.take_used().len(), 1, "a app apaga a usada da base");
        // B sem fichas: so voz pelo relay, video em pausa pelo motivo certo.
        assert_eq!(b_to_a, 0);
        assert_eq!(b.video_params().paused, Some(Pause::RelayLimit), "{:?}", b.video_params());
        assert_eq!((vb.fichas_spent, vb.relay_mode), (0, 2));
        assert!(audio_a > 350 && audio_b > 350, "a voz passa nos dois sentidos ({audio_a}/{audio_b})");
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ligacao_direta_nao_gasta_fichas() {
    let (r, _) = relay(true).await;
    let (ma, mb, room) = sessions();
    let (wa, wb) = (wallet(2), MemWallet::new(Vec::new()));
    let a = start(&r.udp, &r.public, ma, Role::Caller, &room, Some(wa.clone()));
    let b = start(&r.udp, &r.public, mb, Role::Callee, &room, Some(wb.clone()));
    tokio::task::spawn_blocking(move || {
        wait_for(5, "na sala", || a.stats().joined && b.stats().joined);
        go_direct(&a, &b);
        let (a_to_b, b_to_a, _, _) = video_call(&a, &b, 6);
        eprintln!("DIRETO: A->B {a_to_b}, B->A {b_to_a}, fichas gastas {}", a.video_stats().fichas_spent);
        assert!(a_to_b >= 100 && b_to_a >= 100, "video direto nos dois sentidos, mesmo sem fichas ({a_to_b}/{b_to_a})");
        assert_eq!((a.video_stats().fichas_spent, wa.left()), (0, 2), "nada gasto");
        assert!(b.video_params().paused.is_none(), "{:?}", b.video_params());
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ficha_ja_gasta_passa_a_seguinte() {
    let (r, spent) = relay(true).await;
    let (ma, mb, room) = sessions();
    let f = fichas_n(2);
    // A primeira ja foi gasta (a app morreu antes de a apagar da base).
    assert!(spent.mark(f[0].epoch, &fichas::spent_id(&f[0].nonce)).await.unwrap());
    let wa = MemWallet::new(f.clone());
    let a = start(&r.udp, &r.public, ma, Role::Caller, &room, Some(wa.clone()));
    let b = start(&r.udp, &r.public, mb, Role::Callee, &room, None);
    tokio::task::spawn_blocking(move || {
        wait_for(5, "na sala", || a.stats().joined && b.stats().joined);
        b.video_set_receiving(true);
        let (a_to_b, _, _, _) = video_call(&a, &b, 6);
        assert!(a_to_b >= 80, "{a_to_b}");
        let mut used = wa.take_used();
        used.sort();
        let mut both = vec![f[0].nonce, f[1].nonce];
        both.sort();
        assert_eq!((used, wa.left(), a.video_stats().fichas_spent), (both, 0, 1));
    })
    .await
    .unwrap();
}

/// Particao de chaves: o site publica outra chave (ou a app nunca a obteve, site indisponivel).
/// Nada se gasta, o video pelo relay pausa, a voz continua e o direto continua com video.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chave_diferente_da_publicada_ou_sem_site_falha_fechada() {
    let e = fichas::current_epoch();
    let other = vec![(e, Issuer::derive(&[0x12; 32], e).unwrap().public_key())];
    for published in [other, Vec::new()] {
        let (r, _) = relay(true).await;
        let (ma, mb, room) = sessions();
        let wa = wallet(2);
        let a = start_with(&r.udp, &r.public, ma, Role::Caller, &room, Some(wa.clone()), published.clone());
        let b = start(&r.udp, &r.public, mb, Role::Callee, &room, None);
        let empty = published.is_empty();
        tokio::task::spawn_blocking(move || {
            wait_for(5, "na sala", || a.stats().joined && b.stats().joined);
            let (a_to_b, _, _, audio_b) = video_call(&a, &b, 5);
            assert_eq!(a_to_b, 0, "sem site: {empty}");
            assert_eq!(a.video_params().paused, Some(Pause::RelayLimit));
            assert_eq!((wa.left(), a.video_stats().fichas_spent), (2, 0), "nada gasto");
            assert!(audio_b > 200, "a voz passa ({audio_b})");
            go_direct(&a, &b);
            let (a_to_b, _, _, _) = video_call(&a, &b, 4);
            assert!(a_to_b >= 60, "direto continua com video ({a_to_b})");
        })
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_sem_fichas_video_como_antes() {
    let (r, _) = relay(false).await;
    let (ma, mb, room) = sessions();
    let a = start(&r.udp, &r.public, ma, Role::Caller, &room, None);
    let b = start(&r.udp, &r.public, mb, Role::Callee, &room, None);
    tokio::task::spawn_blocking(move || {
        wait_for(5, "na sala", || a.stats().joined && b.stats().joined);
        let (a_to_b, b_to_a, _, _) = video_call(&a, &b, 6);
        assert!(a_to_b >= 100 && b_to_a >= 100, "{a_to_b}/{b_to_a}");
        assert_eq!(a.video_stats().fichas_spent, 0);
    })
    .await
    .unwrap();
}

/// Relay antigo (antes do 0006): nao responde ao pedido da chave nem as fichas. O motor
/// desiste ao fim de ~2 s e envia como antes (um relay antigo nao pede fichas).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_antigo_que_nao_responde_a_chave() {
    let (r, _) = relay(false).await;
    let pa = proxy_without_fichas(&r.udp).await;
    let pb = proxy_without_fichas(&r.udp).await;
    let (ma, mb, room) = sessions();
    let wa = wallet(1);
    let a = start(&pa, &r.public, ma, Role::Caller, &room, Some(wa.clone()));
    let b = start(&pb, &r.public, mb, Role::Callee, &room, None);
    tokio::task::spawn_blocking(move || {
        wait_for(5, "na sala", || a.stats().joined && b.stats().joined);
        let (a_to_b, _, _, _) = video_call(&a, &b, 7);
        assert!(a_to_b >= 80, "{a_to_b}");
        assert_eq!(wa.left(), 1, "nada gasto");
    })
    .await
    .unwrap();
}

/// Proxy UDP para o relay que deita fora os pacotes das fichas (como um relay antigo).
async fn proxy_without_fichas(relay: &str) -> String {
    let front = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let back = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    back.connect(relay).await.unwrap();
    let client = Arc::new(Mutex::new(None::<SocketAddr>));
    let stop = Arc::new(AtomicBool::new(false));
    {
        let (front, back, client) = (front.clone(), back.clone(), client.clone());
        tokio::spawn(async move {
            let mut buf = vec![0u8; 2048];
            loop {
                let Ok((n, from)) = front.recv_from(&mut buf).await else { continue };
                *client.lock().unwrap() = Some(from);
                if n > 0 && (buf[0] == fichas::KEY_REQ_KIND || buf[0] == fichas::REDEEM_KIND) {
                    continue;
                }
                let _ = back.send(&buf[..n]).await;
            }
        });
    }
    {
        let (front, back, client, stop) = (front.clone(), back.clone(), client.clone(), stop.clone());
        tokio::spawn(async move {
            let mut buf = vec![0u8; 2048];
            while !stop.load(Relaxed) {
                let Ok(n) = back.recv(&mut buf).await else { continue };
                let to = *client.lock().unwrap();
                if let Some(c) = to {
                    let _ = front.send_to(&buf[..n], c).await;
                }
            }
        });
    }
    front.local_addr().unwrap().to_string()
}
