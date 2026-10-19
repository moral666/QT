//! O motor de uma chamada: liga o microfone ao relay das chamadas e o relay ao
//! altifalante.
//!
//!   push_capture(20 ms)  -> Opus -> cifra (MediaSession) -> fila -> rede
//!   rede -> decifra -> jitter buffer;  pull_playback(20 ms) -> jitter -> Opus/DRED/PLC
//!
//! A REDE corre numa thread propria (tokio, uma thread): entra na sala do relay com o
//! pedido Noise_N (repetido a cada 500 ms ate a resposta, e a cada 10 s depois, que
//! tambem serve para medir o tempo ate ao relay), envia e recebe os pacotes. Se o UDP
//! nao responder em 2 s, passa para a reserva `wss://<relay>/call` (feature `ws`).
//!
//! As chaves nunca saem daqui: a app entrega a `MediaSession` (fase 1) e o pedido de
//! entrada ja feito (`QtClient::call_join_packet`).

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use qt_core::call::{join_payload, parse_join_ok, MediaSession, Role, RoomKeys, JOIN_KIND, JOIN_OK_KIND, JOIN_OK_LEN, JOIN_PROLOGUE};
use tokio::sync::{mpsc, oneshot};

use crate::codec::{Decoder, Encoder, EncoderConfig, FRAME_SAMPLES, MAX_OPUS_BYTES};
use crate::jitter::{JitterBuffer, JitterStats, Slot};

/// Se o UDP nao der resposta em tanto tempo, tenta a reserva por WebSocket.
const UDP_GIVE_UP: Duration = Duration::from_secs(2);
const JOIN_RETRY: Duration = Duration::from_millis(500);
const WS_RETRY: Duration = Duration::from_secs(5);
const JOIN_KEEPALIVE: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("codec: {0}")]
    Codec(#[from] crate::codec::CodecError),
    #[error("cifra do pacote falhou")]
    Seal,
    #[error("nao foi possivel arrancar a thread da rede")]
    Thread,
    #[error("chave do relay invalida")]
    RelayKey,
}

/// Onde esta o relay das chamadas.
#[derive(Debug, Clone)]
pub struct RelayTarget {
    /// "relay.sealer.pt:443"
    pub udp: String,
    /// "wss://relay.sealer.pt/call" (reserva quando o UDP esta bloqueado).
    pub ws_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub relay: RelayTarget,
    pub encoder: EncoderConfig,
    /// Usar logo a reserva por WebSocket (testes e redes conhecidas por bloquear UDP).
    pub force_ws: bool,
    /// So para testes e para a ferramenta do PC: deita fora esta % dos pacotes recebidos.
    pub simulated_loss_percent: u32,
}

impl EngineConfig {
    pub fn new(relay: RelayTarget) -> Self {
        EngineConfig { relay, encoder: EncoderConfig::default(), force_ws: false, simulated_loss_percent: 0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transport {
    #[default]
    Connecting,
    Udp,
    WebSocket,
}

#[derive(Debug, Clone, Default)]
pub struct EngineStats {
    pub transport: Transport,
    pub joined: bool,
    /// O outro telemovel ja esta na sala.
    pub peer_present: bool,
    /// Ida e volta ate ao relay (pedido de entrada -> resposta), em ms.
    pub relay_rtt_ms: Option<u32>,
    pub sent: u64,
    /// Media de bytes Opus por frame enviado.
    pub avg_opus_bytes: u32,
    pub received: u64,
    pub invalid: u64,
    pub simulated_drops: u64,
    /// Frames reconstruidos pelo DRED e preenchidos pelo PLC.
    pub dred_frames: u64,
    pub plc_frames: u64,
    pub jitter: JitterStats,
}

struct Playback {
    jitter: JitterBuffer,
    decoder: Decoder,
    /// Pacote de onde veio o DRED ja lido (para nao o ler outra vez no mesmo buraco).
    dred_from: Option<u64>,
}

struct Shared {
    media: Mutex<MediaSession>,
    encoder: Mutex<Encoder>,
    playback: Mutex<Playback>,
    stats: Mutex<EngineStats>,
    opus_bytes: Mutex<u64>,
    room: RoomKeys,
}

pub struct CallEngine {
    shared: Arc<Shared>,
    outbox: mpsc::Sender<Vec<u8>>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

/// Pedido para entrar na sala (o mesmo que `QtClient::call_join_packet` faz).
pub fn join_packet(relay_public_key: &[u8], role: Role, room: &RoomKeys) -> Result<Vec<u8>, EngineError> {
    let msg = qt_transport::noise_n_seal(relay_public_key, JOIN_PROLOGUE, &join_payload(role, room)).map_err(|_| EngineError::RelayKey)?;
    let mut p = Vec::with_capacity(1 + msg.len());
    p.push(JOIN_KIND);
    p.extend_from_slice(&msg);
    Ok(p)
}

impl CallEngine {
    /// Arranca a chamada: entra na sala e fica pronto para `push_capture` / `pull_playback`.
    pub fn start(cfg: EngineConfig, media: MediaSession, join: Vec<u8>) -> Result<Self, EngineError> {
        let room = RoomKeys { room_id: media.room_id(), relay_key: media.relay_key() };
        let shared = Arc::new(Shared {
            media: Mutex::new(media),
            encoder: Mutex::new(Encoder::new(cfg.encoder)?),
            playback: Mutex::new(Playback { jitter: JitterBuffer::new(), decoder: Decoder::new()?, dred_from: None }),
            stats: Mutex::new(EngineStats::default()),
            opus_bytes: Mutex::new(0),
            room,
        });
        let (outbox, rx) = mpsc::channel(64);
        let (stop, stop_rx) = oneshot::channel();
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("qt-call-net".into())
            .spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
                rt.block_on(net::run(s, cfg, join, rx, stop_rx));
            })
            .map_err(|_| EngineError::Thread)?;
        Ok(CallEngine { shared, outbox, stop: Some(stop), thread: Some(thread) })
    }

    /// 20 ms do microfone (960 amostras, 48 kHz, mono). Codifica, cifra e envia.
    pub fn push_capture(&self, pcm: &[i16]) -> Result<(), EngineError> {
        let mut buf = [0u8; MAX_OPUS_BYTES];
        let n = self.shared.encoder.lock().map_err(|_| EngineError::Seal)?.encode(pcm, &mut buf)?;
        let packet = self.shared.media.lock().map_err(|_| EngineError::Seal)?.seal(&buf[..n]).map_err(|_| EngineError::Seal)?;
        // Fila cheia (rede parada): o frame perde-se, como na rede.
        let _ = self.outbox.try_send(packet);
        if let (Ok(mut st), Ok(mut bytes)) = (self.shared.stats.lock(), self.shared.opus_bytes.lock()) {
            st.sent += 1;
            *bytes += n as u64;
            st.avg_opus_bytes = (*bytes / st.sent) as u32;
        }
        Ok(())
    }

    /// Os proximos 20 ms para o altifalante (960 amostras, 48 kHz, mono).
    pub fn pull_playback(&self, out: &mut [i16]) -> Result<(), EngineError> {
        if out.len() < FRAME_SAMPLES {
            return Err(crate::codec::CodecError::FrameSize.into());
        }
        let out = &mut out[..FRAME_SAMPLES];
        let Ok(mut pb) = self.shared.playback.lock() else {
            out.fill(0);
            return Ok(());
        };
        let pb = &mut *pb;
        let (mut dred, mut plc) = (0, 0);
        match pb.jitter.pop(Instant::now()) {
            Slot::Waiting => out.fill(0),
            Slot::Frame(f) if f.is_empty() => out.fill(0),
            Slot::Frame(f) => {
                if pb.decoder.decode(&f, out).is_err() {
                    pb.decoder.conceal(out)?;
                    plc += 1;
                }
            }
            Slot::Lost { ahead_counter, ahead, frames_back } => {
                if pb.dred_from != Some(ahead_counter) {
                    pb.decoder.parse_dred(&ahead, frames_back);
                    pb.dred_from = Some(ahead_counter);
                }
                if pb.decoder.decode_dred(frames_back, out)? {
                    dred += 1;
                } else {
                    plc += 1;
                }
            }
            Slot::Conceal => {
                pb.decoder.conceal(out)?;
                plc += 1;
            }
        }
        let jitter = pb.jitter.stats();
        if let Ok(mut st) = self.shared.stats.lock() {
            st.dred_frames += dred;
            st.plc_frames += plc;
            st.jitter = jitter;
        }
        Ok(())
    }

    pub fn stats(&self) -> EngineStats {
        self.shared.stats.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Desliga: para a rede e espera pela thread.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for CallEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

mod net {
    use super::*;
    use std::net::SocketAddr;
    use tokio::net::UdpSocket;

    /// Pseudo-aleatorio barato para simular perdas (nao e criptografico).
    struct Rng(u64);
    impl Rng {
        fn percent(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % 100) as u32
        }
    }

    enum Incoming {
        Udp(Vec<u8>),
        #[cfg_attr(not(feature = "ws"), allow(dead_code))]
        Ws(Vec<u8>),
        #[cfg_attr(not(feature = "ws"), allow(dead_code))]
        WsClosed,
    }

    struct State {
        joined_udp: bool,
        joined_ws: bool,
        last_join: Option<Instant>,
        /// Quando tentar (outra vez) a reserva por WebSocket.
        ws_next_try: Instant,
    }

    fn set_stats(shared: &Shared, f: impl FnOnce(&mut EngineStats)) {
        if let Ok(mut st) = shared.stats.lock() {
            f(&mut st);
        }
    }

    pub(super) async fn run(
        shared: Arc<Shared>,
        cfg: EngineConfig,
        join: Vec<u8>,
        mut outbox: mpsc::Receiver<Vec<u8>>,
        mut stop: oneshot::Receiver<()>,
    ) {
        let (in_tx, mut inbox) = mpsc::channel::<Incoming>(256);
        let udp: Option<Arc<UdpSocket>> = if cfg.force_ws { None } else { open_udp(&cfg.relay.udp).await };
        if let Some(u) = &udp {
            let (u, tx) = (u.clone(), in_tx.clone());
            tokio::spawn(async move {
                let mut buf = vec![0u8; 2048];
                loop {
                    match u.recv(&mut buf).await {
                        Ok(n) => {
                            if tx.send(Incoming::Udp(buf[..n].to_vec())).await.is_err() {
                                break;
                            }
                        }
                        // Ex.: "porta inalcancavel" enquanto o relay reinicia. Continua a ouvir.
                        Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                    }
                }
            });
        }
        #[cfg(feature = "ws")]
        let mut ws: Option<ws::Sink> = None;
        let first_ws = if udp.is_none() { Instant::now() } else { Instant::now() + UDP_GIVE_UP };
        let mut st = State { joined_udp: false, joined_ws: false, last_join: None, ws_next_try: first_ws };
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed | 1);
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = &mut stop => break,
                Some(msg) = inbox.recv() => {
                    let (data, via_ws) = match msg {
                        Incoming::Udp(d) => (d, false),
                        Incoming::Ws(d) => (d, true),
                        Incoming::WsClosed => {
                            #[cfg(feature = "ws")]
                            { ws = None; }
                            st.joined_ws = false;
                            continue;
                        }
                    };
                    on_packet(&shared, &cfg, &mut st, &mut rng, &data, via_ws);
                }
                Some(packet) = outbox.recv() => {
                    // Sem estar na sala, o relay deitava fora: nem se envia.
                    if st.joined_udp {
                        if let Some(u) = &udp { let _ = u.send(&packet).await; }
                    } else if st.joined_ws {
                        #[cfg(feature = "ws")]
                        if let Some(w) = ws.as_mut() { let _ = w.send(packet).await; }
                    }
                }
                _ = tick.tick() => {
                    let joined = st.joined_udp || st.joined_ws;
                    let every = if joined { JOIN_KEEPALIVE } else { JOIN_RETRY };
                    if st.last_join.is_none_or(|t| t.elapsed() >= every) {
                        st.last_join = Some(Instant::now());
                        if let Some(u) = &udp {
                            if !st.joined_ws { let _ = u.send(&join).await; }
                        }
                        #[cfg(feature = "ws")]
                        if let Some(w) = ws.as_mut() {
                            if !st.joined_udp { let _ = w.send(join.clone()).await; }
                        }
                    }
                    // Sem resposta por UDP: reserva por WebSocket (de novo a cada 5 s se falhar
                    // ou se a ligacao cair).
                    #[cfg(feature = "ws")]
                    if !joined && ws.is_none() && Instant::now() >= st.ws_next_try {
                        st.ws_next_try = Instant::now() + WS_RETRY;
                        if let Some(url) = &cfg.relay.ws_url {
                            if let Some(mut sink) = ws::connect(url, in_tx.clone()).await {
                                let _ = sink.send(join.clone()).await;
                                st.last_join = Some(Instant::now());
                                ws = Some(sink);
                            }
                        }
                    }
                    #[cfg(not(feature = "ws"))]
                    let _ = (st.ws_next_try, WS_RETRY);
                }
            }
        }
    }

    async fn open_udp(target: &str) -> Option<Arc<UdpSocket>> {
        let addr: SocketAddr = tokio::net::lookup_host(target).await.ok()?.next()?;
        let local: SocketAddr = if addr.is_ipv4() { ([0, 0, 0, 0], 0).into() } else { ([0u16; 8], 0).into() };
        let sock = UdpSocket::bind(local).await.ok()?;
        // So aceita pacotes vindos do relay.
        sock.connect(addr).await.ok()?;
        Some(Arc::new(sock))
    }

    fn on_packet(shared: &Shared, cfg: &EngineConfig, st: &mut State, rng: &mut Rng, data: &[u8], via_ws: bool) {
        if data.len() == JOIN_OK_LEN && data[0] == JOIN_OK_KIND {
            if let Some(peer) = parse_join_ok(&shared.room, data) {
                if via_ws {
                    st.joined_ws = true;
                } else {
                    st.joined_udp = true;
                }
                let rtt = st.last_join.map(|t| t.elapsed().as_millis() as u32);
                let transport = if st.joined_udp { Transport::Udp } else { Transport::WebSocket };
                set_stats(shared, |s| {
                    s.joined = true;
                    s.transport = transport;
                    s.peer_present |= peer;
                    if let Some(r) = rtt.filter(|r| *r < 5_000) {
                        s.relay_rtt_ms = Some(r);
                    }
                });
            }
            return;
        }
        if cfg.simulated_loss_percent > 0 && rng.percent() < cfg.simulated_loss_percent {
            set_stats(shared, |s| s.simulated_drops += 1);
            return;
        }
        let opened = shared.media.lock().ok().and_then(|mut m| m.open(data).ok());
        match opened {
            Some((counter, frame)) => {
                if let Ok(mut pb) = shared.playback.lock() {
                    pb.jitter.push(counter, frame, Instant::now());
                }
                set_stats(shared, |s| {
                    s.received += 1;
                    s.peer_present = true;
                });
            }
            None => set_stats(shared, |s| s.invalid += 1),
        }
    }

    #[cfg(feature = "ws")]
    mod ws {
        use super::Incoming;
        use futures_util::stream::SplitSink;
        use futures_util::{SinkExt, StreamExt};
        use tokio::sync::mpsc;
        use tokio_tungstenite::tungstenite::Message;
        use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

        type Stream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

        pub(super) struct Sink(SplitSink<Stream, Message>);

        impl Sink {
            pub(super) async fn send(&mut self, data: Vec<u8>) -> Result<(), ()> {
                self.0.send(Message::Binary(data)).await.map_err(|_| ())
            }
        }

        /// Liga a `wss://.../call` (5 s no maximo). O que chegar vai para `tx`.
        pub(super) async fn connect(url: &str, tx: mpsc::Sender<Incoming>) -> Option<Sink> {
            // TLS: escolhe o fornecedor de criptografia (ring); se ja houver um, fica esse.
            let _ = rustls::crypto::ring::default_provider().install_default();
            let fut = tokio_tungstenite::connect_async(url);
            let (stream, _) = tokio::time::timeout(std::time::Duration::from_secs(5), fut).await.ok()?.ok()?;
            let (sink, mut read) = stream.split();
            tokio::spawn(async move {
                while let Some(Ok(msg)) = read.next().await {
                    if let Message::Binary(b) = msg {
                        if tx.send(Incoming::Ws(b)).await.is_err() {
                            return;
                        }
                    }
                }
                let _ = tx.send(Incoming::WsClosed).await;
            });
            Some(Sink(sink))
        }
    }
}
