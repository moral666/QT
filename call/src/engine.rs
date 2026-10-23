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
//! LIGACAO DIRETA (P2P, fase 5). Desligada por omissao: so o nucleo (`qt_ffi::call`) a
//! liga (`p2p_enable`), depois de atender, com um contacto verificado e sem "sempre pelo
//! relay" em nenhum dos lados. Sem isso o motor nunca recolhe enderecos nem envia nada a
//! ninguem a nao ser ao relay.
//!   * O MESMO socket UDP serve o relay e o caminho direto (dual IPv4/IPv6 quando o
//!     sistema deixa): a porta que o relay ve e a que o outro telemovel vai usar.
//!   * Enderecos: os da rede local (escolha da rota, sem enviar nada) e o IP publico
//!     visto pelo relay (`addr_request`). O nucleo envia-os ao outro lado dentro do ratchet.
//!   * Com os enderecos do outro lado: sondas autenticadas (`PathKeys`) a cada 200 ms
//!     durante 10 s. A primeira resposta valida escolhe o caminho; o audio passa a ir
//!     direto (no primeiro segundo ainda vai tambem pelo relay).
//!   * Caminho em uso: uma sonda a cada 100 ms. Cada pacote sai pelos DOIS caminhos
//!     (direto + relay), sem silencio enquanto se decide, quando:
//!       - falta resposta as sondas ha mais de 250 ms (o MEU sentido falhou), ou
//!       - o audio do outro lado vinha pelo direto e parou ha mais de 150 ms (50 pacotes/s:
//!         a falta deles ve-se antes das sondas; quase sempre os dois sentidos caem juntos).
//!   * O audio recebido NAO conta como "o direto esta vivo": so prova o sentido dele para
//!     mim. Se so o meu sentido falhar, sao as sondas que o apanham.
//!   * Volta a so direto quando as respostas regressam e ficam boas 1 s (2 s se o caminho
//!     tinha morrido). Sem resposta em 3 s: so relay, e uma sonda por segundo para voltar.
//!     O relay nunca deixa a sala (os pedidos de entrada a cada 10 s mantem-na aberta).
//!   * A rececao aceita os dois caminhos ao mesmo tempo: o contador e a janela
//!     anti-repeticao sao os mesmos, e a copia repetida e descartada (`duplicates`).
//!   * Audio vindo de fora do relay so e aceite de enderecos que ja mandaram uma sonda
//!     valida (nada de decifrar pacotes de qualquer um).
//!
//! VIDEO (fase 5a, `crate::video`). Um fluxo a mais pelo MESMO caminho do audio (direto,
//! relay ou os dois na duplicacao). O nucleo diz quando se pode enviar (camara ligada E o
//! outro aceitou: `video_set_sending`) e quando se aceita receber (`video_set_receiving`);
//! sem isso nada sai nem se processa. As imagens ja codificadas (H.264, da app) sao
//! partidas em pacotes de ate 1225 bytes e passam por um ritmador (5 ms) ao debito que o
//! controlo de debito decide; o audio nunca espera por ele. Fora da ligacao direta, o
//! debito desce logo ao teto do relay. Quem recebe remonta as imagens, espera pelos
//! pedacos atrasados (a copia pelo relay), mostra cada imagem quando o audio que esta a
//! tocar chega ao momento em que foi captada, pede uma imagem completa quando perde uma, e
//! manda um relatorio a cada 250 ms (perdas, atraso de fila, perdas no AUDIO, rede).
//!
//! As chaves nunca saem daqui: a app entrega a `MediaSession` (fase 1) e o pedido de
//! entrada ja feito (`QtClient::call_join_packet`).

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use qt_core::call::{join_payload, parse_join_ok, MediaSession, Role, RoomKeys, VideoSession, JOIN_KIND, JOIN_OK_KIND, JOIN_OK_LEN, JOIN_PROLOGUE};
use tokio::sync::{mpsc, oneshot};

use crate::codec::{Decoder, Encoder, EncoderConfig, FRAME_SAMPLES, MAX_OPUS_BYTES};
use crate::jitter::{JitterBuffer, JitterStats, Slot};
use crate::video::{fragment, RateController, VideoReceiver, VideoRecvStats, MAX_FRAME_BYTES};
pub use crate::video::{Net, Pause, ReceivedFrame, VideoParams};

/// Se o UDP nao der resposta em tanto tempo, tenta a reserva por WebSocket.
const UDP_GIVE_UP: Duration = Duration::from_secs(2);
const JOIN_RETRY: Duration = Duration::from_millis(500);
const WS_RETRY: Duration = Duration::from_secs(5);
const JOIN_KEEPALIVE: Duration = Duration::from_secs(10);

/// Maximo de enderecos de cada lado (os meus e os do outro).
pub const MAX_CANDIDATES: usize = 8;

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
    /// So para testes: aceita enderecos 127.0.0.1 / ::1 na ligacao direta.
    pub p2p_allow_loopback: bool,
    /// So para testes: deita fora esta % do audio recebido pelo caminho direto (as sondas passam).
    pub simulated_direct_loss_percent: u32,
    /// So para testes: com caminho direto, envia sempre tambem pelo relay.
    pub test_both_paths: bool,
}

impl EngineConfig {
    pub fn new(relay: RelayTarget) -> Self {
        EngineConfig {
            relay,
            encoder: EncoderConfig::default(),
            force_ws: false,
            simulated_loss_percent: 0,
            p2p_allow_loopback: false,
            simulated_direct_loss_percent: 0,
            test_both_paths: false,
        }
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
    /// O audio que envio vai agora direto ao outro telemovel.
    pub direct: bool,
    /// ... e tambem pelo relay (o caminho direto falhou respostas ha pouco: ainda a decidir).
    pub both_paths: bool,
    /// Pacotes de audio recebidos pelo caminho direto.
    pub received_direct: u64,
    /// Copias repetidas descartadas (o mesmo pacote pelos dois caminhos). Nao contam como
    /// recebidas, perdidas nem atrasadas: nem chegam ao jitter buffer.
    pub duplicates: u64,
    /// Sondas do caminho direto enviadas (para ver o gasto: 5/s com o caminho em uso, 1/s
    /// com ele em baixo).
    pub probes_sent: u64,
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

/// Video desta chamada: contas de quem envia e de quem recebe.
#[derive(Debug, Clone, Default)]
pub struct VideoStats {
    pub sent_frames: u64,
    pub sent_packets: u64,
    pub sent_bytes: u64,
    /// Imagens deitadas fora antes de sair (fila acima de ~0,4 s: a rede nao da).
    pub dropped_frames: u64,
    pub queued_bytes: u64,
    pub recv: VideoRecvStats,
    /// Copias repetidas descartadas (o mesmo pacote pelos dois caminhos).
    pub duplicates: u64,
    pub invalid: u64,
    pub keyframe_requests_received: u64,
    /// Ultimo aviso do relay: 0 normal, 1 so 240p, 2 sem video pelo relay.
    pub relay_mode: u8,
    /// Perda do meu video medida pelo outro lado (ultimo relatorio, em milesimas).
    pub peer_loss_permille: u32,
    /// Relatorios recebidos e, desses, os que diziam "nada esta a chegar".
    pub feedbacks: u64,
    pub stalls: u64,
}

struct VideoState {
    session: Option<VideoSession>,
    /// Camara ligada E o outro aceitou (decidido pelo nucleo).
    sending: bool,
    /// Aceitei o video do outro E ele tem a camara ligada.
    receiving: bool,
    next_frame_id: u32,
    /// Conteudos a espera do ritmador, AINDA POR CIFRAR: o contador so avanca com pacotes
    /// que saem (um pacote deitado fora da fila nao pode parecer perdido ao outro lado).
    queue: VecDeque<Vec<u8>>,
    /// Bytes que esses conteudos vao ocupar ja cifrados.
    queued_bytes: usize,
    /// Contadores dos pacotes enviados no ultimo ~2 s (para saber se nada esta a chegar).
    sent_log: VecDeque<(u64, Instant)>,
    rx: VideoReceiver,
    ctl: RateController,
    net: Net,
    /// O descodificador falhou: pedir ao outro lado uma imagem completa.
    want_remote_keyframe: bool,
    stats: VideoStats,
}

/// Um pacote enviado ha mais disto ja devia aparecer no relatorio (intervalo de 250 ms e
/// ida e volta ate ~550 ms).
const STALL_AGE: Duration = Duration::from_millis(800);

impl VideoState {
    /// Nada esta a chegar: ha pacotes enviados ha mais de `STALL_AGE` acima do maior
    /// contador que o outro lado diz ter recebido.
    fn stalled(&self, highest: Option<u64>, now: Instant) -> bool {
        self.sent_log
            .iter()
            .any(|(c, t)| now.saturating_duration_since(*t) > STALL_AGE && highest.is_none_or(|h| *c > h))
    }

    /// Cifra um conteudo e regista o contador (so para pacotes que vao sair ja).
    fn seal_now(&mut self, body: &[u8], now: Instant) -> Option<Vec<u8>> {
        let p = self.session.as_mut()?.seal(body).ok()?;
        let counter = u64::from_be_bytes(p.get(9..17)?.try_into().ok()?);
        self.sent_log.push_back((counter, now));
        while self.sent_log.front().is_some_and(|(_, t)| now.saturating_duration_since(*t) > Duration::from_secs(2)) {
            self.sent_log.pop_front();
        }
        self.stats.sent_packets += 1;
        self.stats.sent_bytes += p.len() as u64;
        Some(p)
    }
}

/// Linhas de tempo do audio, para sincronizar o video.
#[derive(Default)]
struct AvClock {
    /// Ultimo pacote de audio enviado: (quando, contador).
    send: Option<(Instant, u64)>,
    /// Contador do audio do outro lado que esta a tocar agora.
    play: Option<u64>,
}

struct Playback {
    jitter: JitterBuffer,
    decoder: Decoder,
    /// Pacote de onde veio o DRED ja lido (para nao o ler outra vez no mesmo buraco).
    dred_from: Option<u64>,
}

/// O que o nucleo decidiu sobre a ligacao direta (lido pela thread da rede a cada 100 ms).
#[derive(Default)]
struct P2pCtl {
    enabled: bool,
    /// Os meus enderecos, depois de `p2p_enable`. `None`: ainda a recolher.
    local: Option<Vec<SocketAddr>>,
    /// Os enderecos do outro lado (vieram cifrados no ratchet). `None`: ainda nao chegaram.
    peer: Option<Vec<SocketAddr>>,
}

struct Shared {
    /// Chaves do audio: so existem depois de atender (antes disso ja se entra na sala).
    media: Mutex<Option<MediaSession>>,
    encoder: Mutex<Encoder>,
    playback: Mutex<Playback>,
    stats: Mutex<EngineStats>,
    opus_bytes: Mutex<u64>,
    p2p: Mutex<P2pCtl>,
    video: Mutex<VideoState>,
    /// Acorda quem espera por uma imagem (`video_pull`).
    video_cv: Condvar,
    clock: Mutex<AvClock>,
    epoch: Instant,
    /// So para testes: deita fora tudo o que chega pelo direto (sondas incluidas), sem deixar
    /// de enviar: o caminho falha so num sentido.
    test_drop_direct_rx: std::sync::atomic::AtomicBool,
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
    /// Arranca a chamada ja atendida: entra na sala e fica pronto para `push_capture` /
    /// `pull_playback`.
    pub fn start(cfg: EngineConfig, media: MediaSession, join: Vec<u8>) -> Result<Self, EngineError> {
        let room = RoomKeys { room_id: media.room_id(), relay_key: media.relay_key() };
        let e = Self::start_pending(cfg, room, join)?;
        e.attach_media(media);
        Ok(e)
    }

    /// Entra ja na sala (pre-aquecimento: enquanto o telemovel toca), sem as chaves do
    /// audio. O audio comeca quando chegarem (`attach_media`, ao atender).
    pub fn start_pending(cfg: EngineConfig, room: RoomKeys, join: Vec<u8>) -> Result<Self, EngineError> {
        let shared = Arc::new(Shared {
            media: Mutex::new(None),
            encoder: Mutex::new(Encoder::new(cfg.encoder)?),
            playback: Mutex::new(Playback { jitter: JitterBuffer::new(), decoder: Decoder::new()?, dred_from: None }),
            stats: Mutex::new(EngineStats::default()),
            opus_bytes: Mutex::new(0),
            p2p: Mutex::new(P2pCtl::default()),
            video: Mutex::new(VideoState {
                session: None,
                sending: false,
                receiving: false,
                next_frame_id: 0,
                queue: VecDeque::new(),
                queued_bytes: 0,
                sent_log: VecDeque::new(),
                rx: VideoReceiver::new(Instant::now()),
                ctl: RateController::new(),
                net: Net::Unknown,
                want_remote_keyframe: false,
                stats: VideoStats::default(),
            }),
            video_cv: Condvar::new(),
            clock: Mutex::new(AvClock::default()),
            epoch: Instant::now(),
            test_drop_direct_rx: std::sync::atomic::AtomicBool::new(false),
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

    /// As chaves do audio (a chamada foi atendida).
    pub fn attach_media(&self, media: MediaSession) {
        if let Ok(mut v) = self.shared.video.lock() {
            v.session = Some(media.video());
        }
        if let Ok(mut m) = self.shared.media.lock() {
            *m = Some(media);
        }
    }

    /// Ja ha chaves do audio (a chamada foi atendida)?
    pub fn has_media(&self) -> bool {
        self.shared.media.lock().map(|m| m.is_some()).unwrap_or(false)
    }

    /// LIGACAO DIRETA: comecar a recolher os meus enderecos. So o nucleo chama isto (ver
    /// o inicio do ficheiro); sem chaves do audio (antes de atender) nao faz nada.
    pub fn p2p_enable(&self) {
        if !self.has_media() {
            return;
        }
        if let Ok(mut p) = self.shared.p2p.lock() {
            p.enabled = true;
        }
    }

    /// Desliga a ligacao direta ate ao fim da chamada (o audio volta ao relay).
    pub fn p2p_disable(&self) {
        if let Ok(mut p) = self.shared.p2p.lock() {
            *p = P2pCtl::default();
        }
    }

    /// So para testes: o caminho direto deixa de funcionar no sentido de quem chega a este
    /// motor (tudo o que chega pelo direto e deitado fora; o envio continua).
    pub fn test_drop_direct_rx(&self, on: bool) {
        self.shared.test_drop_direct_rx.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Os meus enderecos, para o nucleo os enviar ao outro lado. `None`: ainda a recolher
    /// (ou ligacao direta desligada). Pode ser uma lista vazia (sem UDP).
    pub fn p2p_local_candidates(&self) -> Option<Vec<SocketAddr>> {
        self.shared.p2p.lock().ok().and_then(|p| if p.enabled { p.local.clone() } else { None })
    }

    /// Os enderecos do outro lado (o nucleo ja os decifrou e validou). So contam uma vez
    /// por chamada e so com a ligacao direta ligada.
    pub fn p2p_set_peer(&self, peer: Vec<SocketAddr>) {
        if let Ok(mut p) = self.shared.p2p.lock() {
            if p.enabled && p.peer.is_none() {
                p.peer = Some(peer.into_iter().take(MAX_CANDIDATES).collect());
            }
        }
    }

    /// 20 ms do microfone (960 amostras, 48 kHz, mono). Codifica, cifra e envia.
    pub fn push_capture(&self, pcm: &[i16]) -> Result<(), EngineError> {
        if !self.has_media() {
            return Ok(());
        }
        let mut buf = [0u8; MAX_OPUS_BYTES];
        let n = self.shared.encoder.lock().map_err(|_| EngineError::Seal)?.encode(pcm, &mut buf)?;
        let packet = match self.shared.media.lock().map_err(|_| EngineError::Seal)?.as_mut() {
            Some(m) => m.seal(&buf[..n]).map_err(|_| EngineError::Seal)?,
            None => return Ok(()),
        };
        // Linha de tempo do audio (o video e marcado nela).
        if let (Ok(mut c), Some(counter)) = (self.shared.clock.lock(), packet.get(9..17).and_then(|b| b.try_into().ok())) {
            c.send = Some((Instant::now(), u64::from_be_bytes(counter)));
        }
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
        let playing = pb.jitter.playing();
        if let Ok(mut c) = self.shared.clock.lock() {
            c.play = playing;
        }
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

    // ------------------------------------------------------------------ video

    /// O nucleo: camara ligada E o outro aceitou ver (ou deixou de ser assim). So com isto
    /// sai video deste telemovel.
    pub fn video_set_sending(&self, on: bool) {
        let now = Instant::now();
        if let Ok(mut v) = self.shared.video.lock() {
            let on = on && v.session.is_some();
            v.sending = on;
            v.ctl.set_sending(on, now);
            if !on {
                v.queue.clear();
                v.queued_bytes = 0;
            }
        }
    }

    /// O nucleo: aceitei o video do outro E ele tem a camara ligada. Sem isto o video que
    /// chegue e deitado fora sem ser lido.
    pub fn video_set_receiving(&self, on: bool) {
        if let Ok(mut v) = self.shared.video.lock() {
            if on && !v.receiving {
                v.rx.reset(Instant::now());
            }
            v.receiving = on;
        }
        self.shared.video_cv.notify_all();
    }

    /// Rede deste telemovel (Wi-Fi ou dados moveis): muda o teto da ligacao direta.
    pub fn set_network(&self, n: Net) {
        if let Ok(mut v) = self.shared.video.lock() {
            v.net = n;
            v.ctl.set_local_net(n);
        }
    }

    /// O que o codificador deve fazer agora (a app pergunta a cada ~250 ms).
    pub fn video_params(&self) -> VideoParams {
        self.shared.video.lock().map(|v| v.ctl.params()).unwrap_or_else(|_| RateController::new().params())
    }

    /// O descodificador falhou: pede ao outro lado uma imagem completa.
    pub fn video_request_keyframe(&self) {
        if let Ok(mut v) = self.shared.video.lock() {
            v.want_remote_keyframe = true;
        }
    }

    /// Uma imagem H.264 ja codificada (com SPS/PPS antes das imagens completas). `rotation`:
    /// quartos de volta para a mostrar direita; `age_ms`: ha quanto tempo foi captada.
    /// `false`: nao saiu (sem autorizacao, em pausa, demasiado grande ou fila cheia).
    pub fn video_push(&self, frame: &[u8], keyframe: bool, rotation: u8, age_ms: u32) -> bool {
        let now = Instant::now();
        if frame.is_empty() || frame.len() > MAX_FRAME_BYTES {
            return false;
        }
        let captured = now.checked_sub(Duration::from_millis(age_ms.min(1_000) as u64)).unwrap_or(now);
        let ts = match self.shared.clock.lock().ok().and_then(|c| c.send) {
            Some((t, counter)) => counter as i64 * 20 + (captured.duration_since(t).as_millis() as i64 - t.duration_since(captured).as_millis() as i64),
            None => captured.duration_since(self.shared.epoch).as_millis() as i64,
        }
        .clamp(0, u32::MAX as i64) as u32;
        let Ok(mut v) = self.shared.video.lock() else { return false };
        let p = v.ctl.params();
        if !p.send || !v.sending || v.session.is_none() {
            return false;
        }
        // Fila acima de ~0,4 s ao debito atual: a rede nao da. Deita fora o que esta na fila
        // e, se esta imagem nao for completa, tambem esta (o outro lado ia precisar de uma
        // completa de qualquer forma): pede-se uma ao codificador.
        let max_backlog = ((p.bitrate_bps as usize / 8) * 2 / 5).max(64 * 1024);
        if v.queued_bytes > max_backlog {
            v.queue.clear();
            v.queued_bytes = 0;
            v.stats.dropped_frames += 1;
            if !keyframe {
                v.ctl.request_keyframe();
                return false;
            }
        }
        let id = v.next_frame_id;
        v.next_frame_id = v.next_frame_id.wrapping_add(1);
        for body in fragment(id, keyframe, rotation, ts, frame) {
            v.queued_bytes += qt_core::call::video_packet_len(body.len());
            v.queue.push_back(body);
        }
        v.stats.sent_frames += 1;
        true
    }

    /// A proxima imagem do outro lado para descodificar, quando for a hora dela (sincronizada
    /// com o audio). Espera ate `timeout`. A app chama isto numa thread propria.
    pub fn video_pull(&self, timeout: Duration) -> Option<ReceivedFrame> {
        let deadline = Instant::now() + timeout;
        let mut v = self.shared.video.lock().ok()?;
        loop {
            let now = Instant::now();
            if v.receiving {
                let audio_ts = self.shared.clock.lock().ok().and_then(|c| c.play).map(|c| (c * 20).min(u32::MAX as u64) as u32);
                if let Some(f) = v.rx.pop(now, audio_ts) {
                    return Some(f);
                }
            }
            if now >= deadline {
                return None;
            }
            let wait = (deadline - now).min(Duration::from_millis(10));
            v = self.shared.video_cv.wait_timeout(v, wait).ok()?.0;
        }
    }

    pub fn video_stats(&self) -> VideoStats {
        self.shared
            .video
            .lock()
            .map(|v| {
                let mut s = v.stats.clone();
                s.recv = v.rx.stats.clone();
                s.queued_bytes = v.queued_bytes as u64;
                s
            })
            .unwrap_or_default()
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
    use std::collections::VecDeque;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use tokio::net::UdpSocket;

    use qt_core::call::{
        addr_request, is_probe, is_video, parse_addr_ok, parse_relay_video_notice, CallError, PathKeys, Probe,
        ADDR_OK_KIND, ADDR_OK_LEN, RELAY_VIDEO_KIND, RELAY_VIDEO_LEN,
    };
    use crate::video::{encode_feedback, encode_keyframe_request, parse_body, Body};

    #[cfg(feature = "ws")]
    type WsOpt = Option<ws::Sink>;
    #[cfg(not(feature = "ws"))]
    type WsOpt = ();

    /// Ritmador do video: de 5 em 5 ms.
    const PACE_EVERY: Duration = Duration::from_millis(5);

    /// Recolha dos meus enderecos: ate tanto tempo pelo IP publico visto pelo relay.
    const GATHER_TIMEOUT: Duration = Duration::from_millis(1500);
    const ADDR_RETRY: Duration = Duration::from_millis(400);
    const ADDR_TRIES: u32 = 4;
    /// Sondas a todos os enderecos do outro lado: de 200 em 200 ms durante 10 s.
    const PROBE_EVERY: Duration = Duration::from_millis(200);
    const PROBE_WINDOW: Duration = Duration::from_secs(10);
    /// Recebi uma sonda do outro lado e ainda nunca houve caminho: sondo mais um pouco...
    const PROBE_EXTEND: Duration = Duration::from_secs(3);
    /// ... mas a procura inicial (200 ms a todos os enderecos) nunca passa disto.
    const MAX_DISCOVERY: Duration = Duration::from_secs(30);
    /// Caminho em uso: uma sonda a cada 100 ms (cada resposta mostra que o MEU sentido esta vivo).
    const PATH_PROBE_EVERY: Duration = Duration::from_millis(100);
    /// Caminho morto: uma sonda por segundo, para voltar quando der.
    const DEAD_PROBE_EVERY: Duration = Duration::from_secs(1);
    /// Sem resposta ha mais disto: o audio sai pelos dois caminhos (direto + relay).
    const SUSPECT_AFTER: Duration = Duration::from_millis(250);
    /// O audio do outro lado vinha pelo direto (50 pacotes/s) e parou ha mais disto: tambem
    /// pelos dois. Depois de `RX_GAP_MAX` sem nada ja nao conta (o outro lado passou ao relay
    /// de proposito; as sondas decidem).
    const RX_GAP: Duration = Duration::from_millis(150);
    const RX_GAP_MAX: Duration = Duration::from_secs(1);
    /// Depois de uma falha (ou de um caminho novo), continua pelos dois mais este tempo...
    const DUP_HOLD: Duration = Duration::from_secs(1);
    /// ... ou este, se o caminho tinha morrido.
    const RECOVER_HOLD: Duration = Duration::from_secs(2);
    /// Sem resposta em 3 s: o caminho morreu, so relay.
    const PATH_TIMEOUT: Duration = Duration::from_secs(3);
    /// Sondas que ficam a espera de resposta (txid), e por quanto tempo.
    const PENDING_MAX: usize = 256;
    const PENDING_TTL: Duration = Duration::from_secs(5);
    /// O relogio da rede bate a cada 100 ms: uma sonda marcada para uns microssegundos depois
    /// de um tique sai nesse tique (senao saia so no seguinte, um intervalo inteiro mais tarde).
    const TICK_SLACK: Duration = Duration::from_millis(50);
    /// Sondas recebidas por segundo (cada uma custa um HMAC): o resto deita-se fora.
    const PROBES_PER_SEC: u32 = 200;
    /// Para escolher a rota (e o endereco local) sem enviar nada: enderecos de documentacao
    /// (RFC 5737 / RFC 3849), usados so quando o relay nao tem essa familia.
    const ROUTE_V4: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 443);
    const ROUTE_V6: SocketAddr = SocketAddr::new(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)), 443);

    /// Pseudo-aleatorio barato para simular perdas e para os txid das sondas (as sondas sao
    /// autenticadas: o txid nao precisa de ser secreto).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn percent(&mut self) -> u32 {
            (self.next() % 100) as u32
        }
    }

    enum Incoming {
        /// Do relay (UDP).
        Udp(Vec<u8>),
        /// De outro endereco (o outro telemovel, ou lixo).
        Direct(Vec<u8>, SocketAddr),
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

    /// O socket UDP: o mesmo para o relay e para o caminho direto.
    struct Udp {
        sock: Arc<UdpSocket>,
        /// Endereco do relay (forma canonica: IPv4 como IPv4).
        relay: SocketAddr,
        /// Socket IPv6 que tambem fala IPv4 (`::ffff:a.b.c.d`).
        dual: bool,
        /// Socket so IPv6 (quando nao e dual).
        v6: bool,
    }

    impl Udp {
        fn can_reach(&self, a: &SocketAddr) -> bool {
            self.dual || a.is_ipv6() == self.v6
        }
        fn wire(&self, a: SocketAddr) -> SocketAddr {
            match a.ip() {
                IpAddr::V4(v4) if self.dual => SocketAddr::new(IpAddr::V6(v4.to_ipv6_mapped()), a.port()),
                _ => a,
            }
        }
        async fn send(&self, data: &[u8], to: SocketAddr) {
            let _ = self.sock.send_to(data, self.wire(to)).await;
        }
    }

    pub(super) fn canon(a: SocketAddr) -> SocketAddr {
        SocketAddr::new(a.ip().to_canonical(), a.port())
    }

    /// Endereco que vale a pena dar ao outro lado / sondar (nada de loopback, link-local,
    /// multicast ou documentacao).
    pub(super) fn usable(ip: IpAddr, allow_loopback: bool) -> bool {
        if ip.is_loopback() {
            return allow_loopback;
        }
        match ip.to_canonical() {
            IpAddr::V4(a) => !(a.is_unspecified() || a.is_link_local() || a.is_broadcast() || a.is_multicast() || a.is_documentation()),
            IpAddr::V6(a) => {
                let s = a.segments();
                !(a.is_unspecified() || a.is_multicast() || (s[0] & 0xffc0) == 0xfe80 || (s[0] == 0x2001 && s[1] == 0x0db8))
            }
        }
    }

    /// Endereco local que o sistema usaria para chegar a `dst`. `connect` num socket UDP
    /// so escolhe a rota: nao sai nenhum pacote.
    fn local_ip_towards(dst: SocketAddr) -> Option<IpAddr> {
        let bind: SocketAddr = if dst.is_ipv4() { (Ipv4Addr::UNSPECIFIED, 0).into() } else { (Ipv6Addr::UNSPECIFIED, 0).into() };
        let s = std::net::UdpSocket::bind(bind).ok()?;
        s.connect(dst).ok()?;
        Some(s.local_addr().ok()?.ip())
    }

    /// Por onde sai cada pacote de audio.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Route {
        Relay,
        Direct,
        /// Direto E relay (o outro lado descarta a copia).
        Both,
    }

    struct Gather {
        started: Instant,
        host: Vec<SocketAddr>,
        srflx: Option<SocketAddr>,
        txid: u64,
        next_req: Instant,
        tries: u32,
        done: bool,
    }

    /// A ligacao direta, do lado da thread da rede.
    #[derive(Default)]
    struct Direct {
        enabled: bool,
        keys: Option<PathKeys>,
        gather: Option<Gather>,
        got_peer: bool,
        /// Enderecos do outro lado a sondar (os que vieram no ratchet e os de onde chegaram sondas).
        targets: Vec<SocketAddr>,
        /// Enderecos de onde ja chegou uma sonda valida: so deles se aceita audio direto.
        trusted: Vec<SocketAddr>,
        pending: VecDeque<(u64, Instant)>,
        path: Option<SocketAddr>,
        last_ack: Option<Instant>,
        /// Ultimo pacote de audio valido que chegou pelo direto.
        last_rx: Option<Instant>,
        probe_until: Option<Instant>,
        /// Limite da procura inicial (`MAX_DISCOVERY` depois de chegarem os enderecos).
        discovery_until: Option<Instant>,
        /// De onde veio a ultima sonda do outro lado (com o caminho em baixo, tambem se sonda la).
        last_ping_from: Option<SocketAddr>,
        next_probe: Option<Instant>,
        next_keepalive: Option<Instant>,
        /// Ate quando o audio sai tambem pelo relay depois de uma falha.
        dup_until: Option<Instant>,
        budget: (Option<Instant>, u32),
    }

    impl Direct {
        fn alive(&self, now: Instant) -> bool {
            self.path.is_some() && self.last_ack.is_some_and(|t| now.duration_since(t) < PATH_TIMEOUT)
        }
        /// Por onde sai o proximo pacote (ver o inicio do ficheiro).
        fn route(&mut self, now: Instant) -> Route {
            let (Some(_), Some(t)) = (self.path, self.last_ack) else { return Route::Relay };
            let age = now.duration_since(t);
            if age >= PATH_TIMEOUT {
                return Route::Relay;
            }
            let rx_stopped = self.last_rx.is_some_and(|r| (RX_GAP..RX_GAP_MAX).contains(&now.duration_since(r)));
            if age > SUSPECT_AFTER || rx_stopped {
                let hold = now + DUP_HOLD;
                self.dup_until = Some(self.dup_until.map_or(hold, |u| u.max(hold)));
                return Route::Both;
            }
            if self.dup_until.is_some_and(|u| now < u) {
                Route::Both
            } else {
                Route::Direct
            }
        }
        fn trust(&mut self, a: SocketAddr) {
            if !self.trusted.contains(&a) {
                if self.trusted.len() >= 2 * MAX_CANDIDATES {
                    self.trusted.remove(0);
                }
                self.trusted.push(a);
            }
        }
        fn add_target(&mut self, a: SocketAddr) -> bool {
            if self.targets.contains(&a) || self.targets.len() >= 2 * MAX_CANDIDATES {
                return false;
            }
            self.targets.push(a);
            true
        }
        fn probe_allowed(&mut self, now: Instant) -> bool {
            let (start, n) = &mut self.budget;
            if start.is_none_or(|s| now.duration_since(s) >= Duration::from_secs(1)) {
                *start = Some(now);
                *n = 0;
            }
            *n += 1;
            *n <= PROBES_PER_SEC
        }
    }

    fn set_stats(shared: &Shared, f: impl FnOnce(&mut EngineStats)) {
        if let Ok(mut st) = shared.stats.lock() {
            f(&mut st);
        }
    }

    /// Por onde sai um pacote (audio ou video): direto, relay ou os dois.
    fn route_now(udp: &Option<Arc<Udp>>, cfg: &EngineConfig, d: &mut Direct, now: Instant) -> Route {
        match (udp.is_some(), d.route(now)) {
            (false, _) => Route::Relay,
            (true, Route::Direct) if cfg.test_both_paths => Route::Both,
            (true, r) => r,
        }
    }

    /// Envia um pacote pelo caminho `route`. Sem estar na sala, o relay deitava fora: nem se envia.
    #[cfg_attr(not(feature = "ws"), allow(unused_variables))]
    async fn send_out(udp: &Option<Arc<Udp>>, ws: &mut WsOpt, st: &State, d: &Direct, route: Route, packet: Vec<u8>) {
        if let (Some(u), Some(p), Route::Direct | Route::Both) = (udp, d.path, route) {
            u.send(&packet, p).await;
        }
        if route != Route::Direct {
            if let (Some(u), true) = (udp, st.joined_udp) {
                u.send(&packet, u.relay).await;
            } else if st.joined_ws {
                #[cfg(feature = "ws")]
                if let Some(w) = ws.as_mut() {
                    let _ = w.send(packet).await;
                }
            }
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
        let udp: Option<Arc<Udp>> = if cfg.force_ws { None } else { open_udp(&cfg.relay.udp).await.map(Arc::new) };
        if let Some(u) = &udp {
            let (u, tx) = (u.clone(), in_tx.clone());
            tokio::spawn(async move {
                let mut buf = vec![0u8; 2048];
                loop {
                    match u.sock.recv_from(&mut buf).await {
                        Ok((n, from)) => {
                            let from = canon(from);
                            let msg = if from == u.relay { Incoming::Udp(buf[..n].to_vec()) } else { Incoming::Direct(buf[..n].to_vec(), from) };
                            if tx.send(msg).await.is_err() {
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
        let mut ws: WsOpt = None;
        #[cfg(not(feature = "ws"))]
        let mut ws: WsOpt = ();
        let mut pace = tokio::time::interval(PACE_EVERY);
        pace.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut pacer = Pacer { tokens: 0.0, at: Instant::now() };
        let mut audio_ref = (0u64, 0u64);
        let first_ws = if udp.is_none() { Instant::now() } else { Instant::now() + UDP_GIVE_UP };
        let mut st = State { joined_udp: false, joined_ws: false, last_join: None, ws_next_try: first_ws };
        let mut d = Direct::default();
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed | 1);
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = &mut stop => break,
                Some(msg) = inbox.recv() => {
                    let (data, via_ws) = match msg {
                        Incoming::Udp(x) => (x, false),
                        Incoming::Ws(x) => (x, true),
                        Incoming::Direct(data, from) => {
                            if let Some(u) = &udp {
                                on_direct(&shared, &cfg, u, &mut d, &mut rng, &data, from).await;
                            }
                            continue;
                        }
                        Incoming::WsClosed => {
                            #[cfg(feature = "ws")]
                            { ws = None; }
                            st.joined_ws = false;
                            continue;
                        }
                    };
                    // Resposta "o teu endereco e ..." (so pelo UDP).
                    if !via_ws && data.len() == ADDR_OK_LEN && data[0] == ADDR_OK_KIND {
                        if let (Some(g), Some((txid, addr))) = (d.gather.as_mut(), parse_addr_ok(&shared.room, &data)) {
                            if txid == g.txid && !g.done {
                                g.srflx = Some(addr);
                            }
                        }
                        continue;
                    }
                    on_packet(&shared, &cfg, &mut st, &mut rng, &data, via_ws);
                }
                Some(packet) = outbox.recv() => {
                    // Audio: sai logo, nunca espera pelo video.
                    let route = route_now(&udp, &cfg, &mut d, Instant::now());
                    send_out(&udp, &mut ws, &st, &d, route, packet).await;
                }
                _ = pace.tick() => {
                    let now = Instant::now();
                    let out = pacer.take(&shared, now);
                    if !out.is_empty() {
                        let route = route_now(&udp, &cfg, &mut d, now);
                        for p in out {
                            send_out(&udp, &mut ws, &st, &d, route, p).await;
                        }
                    }
                }
                _ = tick.tick() => {
                    let joined = st.joined_udp || st.joined_ws;
                    let every = if joined { JOIN_KEEPALIVE } else { JOIN_RETRY };
                    if st.last_join.is_none_or(|t| t.elapsed() >= every) {
                        st.last_join = Some(Instant::now());
                        if let Some(u) = &udp {
                            if !st.joined_ws { u.send(&join, u.relay).await; }
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
                    p2p_tick(&shared, &cfg, udp.as_deref(), &st, &mut d, &mut rng).await;
                    let now = Instant::now();
                    let route = route_now(&udp, &cfg, &mut d, now);
                    for p in video_tick(&shared, route == Route::Direct, &mut audio_ref, now) {
                        send_out(&udp, &mut ws, &st, &d, route, p).await;
                    }
                }
            }
        }
    }

    /// A cada 100 ms: o que o nucleo decidiu, recolha dos meus enderecos e sondas.
    async fn p2p_tick(shared: &Shared, cfg: &EngineConfig, udp: Option<&Udp>, st: &State, d: &mut Direct, rng: &mut Rng) {
        let (enabled, peer) = match shared.p2p.lock() {
            Ok(p) => (p.enabled, if d.got_peer { None } else { p.peer.clone() }),
            Err(_) => return,
        };
        if !enabled {
            if d.enabled {
                // Desligada a meio (ex.: "sempre pelo relay"): esquece tudo, o audio volta ao relay.
                *d = Direct::default();
                set_stats(shared, |s| {
                    s.direct = false;
                    s.both_paths = false;
                });
            }
            return;
        }
        d.enabled = true;
        let Some(u) = udp else {
            // So WebSocket: nao ha caminho direto possivel.
            if let Ok(mut p) = shared.p2p.lock() {
                p.local.get_or_insert_with(Vec::new);
            }
            return;
        };
        if d.keys.is_none() {
            d.keys = shared.media.lock().ok().and_then(|m| m.as_ref().map(|m| m.path_keys()));
        }
        let now = Instant::now();

        // 1. Os meus enderecos: os locais (sem enviar nada) e o visto pelo relay.
        if d.gather.is_none() {
            let port = u.sock.local_addr().map(|a| a.port()).unwrap_or(0);
            let mut dsts = Vec::new();
            if u.dual || !u.v6 {
                dsts.push(if u.relay.is_ipv4() { u.relay } else { ROUTE_V4 });
            }
            if u.dual || u.v6 {
                dsts.push(if u.relay.is_ipv6() { u.relay } else { ROUTE_V6 });
            }
            let host = dsts
                .into_iter()
                .filter_map(local_ip_towards)
                .map(|ip| canon(SocketAddr::new(ip, port)))
                .filter(|a| port != 0 && usable(a.ip(), cfg.p2p_allow_loopback))
                .collect();
            d.gather = Some(Gather { started: now, host, srflx: None, txid: rng.next(), next_req: now, tries: 0, done: false });
        }
        if let Some(g) = d.gather.as_mut().filter(|g| !g.done) {
            if g.srflx.is_none() && st.joined_udp && now >= g.next_req && g.tries < ADDR_TRIES {
                g.tries += 1;
                g.next_req = now + ADDR_RETRY;
                u.send(&addr_request(&shared.room, g.txid), u.relay).await;
            }
            if g.srflx.is_some() || now.duration_since(g.started) >= GATHER_TIMEOUT {
                g.done = true;
                let mut local: Vec<SocketAddr> = Vec::new();
                for a in g.srflx.iter().chain(g.host.iter()) {
                    if usable(a.ip(), cfg.p2p_allow_loopback) && !local.contains(a) && local.len() < MAX_CANDIDATES {
                        local.push(*a);
                    }
                }
                if let Ok(mut p) = shared.p2p.lock() {
                    if p.enabled {
                        p.local = Some(local);
                    }
                }
            }
        }

        // 2. Os enderecos do outro lado (uma vez por chamada).
        if let Some(peer) = peer {
            d.got_peer = true;
            for a in peer.into_iter().map(canon) {
                if a != u.relay && u.can_reach(&a) && usable(a.ip(), cfg.p2p_allow_loopback) {
                    d.add_target(a);
                }
            }
            let limit = *d.discovery_until.get_or_insert(now + MAX_DISCOVERY);
            if !d.targets.is_empty() && d.path.is_none() {
                d.probe_until = Some((now + PROBE_WINDOW).min(limit));
            }
        }

        // 3. Sondas.
        let Some(keys) = d.keys.clone() else { return };
        while d.pending.front().is_some_and(|(_, t)| now.duration_since(*t) >= PENDING_TTL) {
            d.pending.pop_front();
        }
        let mut send_to: Vec<SocketAddr> = Vec::new();
        if d.probe_until.is_some_and(|t| now < t) && d.next_probe.is_none_or(|t| now + TICK_SLACK >= t) {
            d.next_probe = Some(now + PROBE_EVERY);
            send_to.extend(d.targets.iter().copied());
        }
        // O caminho em uso (pode nao estar nos `targets`: NAT que muda a porta). Em baixo:
        // so uma sonda por segundo (la e de onde o outro lado sondou por ultimo), ate responder.
        if let Some(p) = d.path {
            if d.next_keepalive.is_none_or(|t| now + TICK_SLACK >= t) {
                let alive = d.alive(now);
                d.next_keepalive = Some(now + if alive { PATH_PROBE_EVERY } else { DEAD_PROBE_EVERY });
                let extra = if alive { None } else { d.last_ping_from.filter(|a| *a != p) };
                for a in std::iter::once(p).chain(extra) {
                    if !send_to.contains(&a) {
                        send_to.push(a);
                    }
                }
            }
        }
        for a in send_to {
            let txid = rng.next();
            if d.pending.len() >= PENDING_MAX {
                d.pending.pop_front();
            }
            d.pending.push_back((txid, now));
            u.send(&keys.probe(Probe::Ping(txid)), a).await;
            set_stats(shared, |s| s.probes_sent += 1);
        }
        let route = d.route(now);
        set_stats(shared, |s| {
            s.direct = route != Route::Relay;
            s.both_paths = route == Route::Both;
        });
    }

    /// Pacote de fora do relay: sonda do outro lado, ou audio por um caminho ja provado.
    async fn on_direct(shared: &Shared, cfg: &EngineConfig, u: &Udp, d: &mut Direct, rng: &mut Rng, data: &[u8], from: SocketAddr) {
        if !d.enabled || shared.test_drop_direct_rx.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let now = Instant::now();
        if is_probe(data) {
            let Some(keys) = d.keys.clone() else { return };
            if !d.probe_allowed(now) {
                return;
            }
            match keys.parse(data) {
                Some(Probe::Ping(txid)) => {
                    u.send(&keys.probe(Probe::Ack(txid)), from).await;
                    d.trust(from);
                    // Endereco novo do outro lado (ex.: NAT que muda a porta): tambem se sonda.
                    if usable(from.ip(), cfg.p2p_allow_loopback) {
                        d.add_target(from);
                        d.last_ping_from = Some(from);
                    }
                    // Ainda a procurar o primeiro caminho: sonda mais um pouco (com limite). Com
                    // um caminho que caiu, fica a 1 sonda/s (bateria) ate ele responder.
                    if d.path.is_none() {
                        let limit = *d.discovery_until.get_or_insert(now + MAX_DISCOVERY);
                        let until = (now + PROBE_EXTEND).min(limit);
                        d.probe_until = Some(d.probe_until.map_or(until, |t| t.max(until)));
                    }
                }
                Some(Probe::Ack(txid)) => {
                    let Some(i) = d.pending.iter().position(|(t, _)| *t == txid) else { return };
                    d.pending.remove(i);
                    d.trust(from);
                    // Fica o primeiro caminho que responde; outro so se este morrer.
                    if d.path == Some(from) || !d.alive(now) {
                        let hold = match (d.path, d.alive(now)) {
                            // Caminho novo: ainda 1 s tambem pelo relay.
                            (None, _) => Some(DUP_HOLD),
                            // Tinha morrido e voltou: 2 s pelos dois antes de confiar.
                            (Some(_), false) => Some(RECOVER_HOLD),
                            _ => None,
                        };
                        if let Some(h) = hold {
                            let until = now + h;
                            d.dup_until = Some(d.dup_until.map_or(until, |u| u.max(until)));
                            // Respondeu: volta ja as sondas de 100 ms (estava a 1/s).
                            d.next_keepalive = Some(now + PATH_PROBE_EVERY);
                        }
                        d.path = Some(from);
                        d.last_ack = Some(now);
                        // Ja ha caminho: acabou a procura a todos os enderecos.
                        d.probe_until = None;
                        set_stats(shared, |s| s.direct = true);
                    }
                }
                None => {}
            }
            return;
        }
        if d.trusted.contains(&from) {
            if cfg.simulated_direct_loss_percent > 0 && rng.percent() < cfg.simulated_direct_loss_percent {
                set_stats(shared, |s| s.simulated_drops += 1);
                return;
            }
            if is_video(data) {
                on_video(shared, data, now);
                return;
            }
            if on_media(shared, cfg, rng, data) {
                d.last_rx = Some(now);
                set_stats(shared, |s| s.received_direct += 1);
            }
        }
    }

    /// Ritmador do video: deixa sair os pacotes ao debito pedido (com 30% de folga), para
    /// as imagens grandes nao sairem de rajada e atropelarem o audio.
    struct Pacer {
        tokens: f64,
        at: Instant,
    }

    impl Pacer {
        fn take(&mut self, shared: &Shared, now: Instant) -> Vec<Vec<u8>> {
            let Ok(mut v) = shared.video.lock() else { return Vec::new() };
            let dt = now.duration_since(self.at).as_secs_f64().min(0.1);
            self.at = now;
            // Em pausa (ou sem autorizacao): o que estava na fila ja nao serve.
            if !v.queue.is_empty() && !(v.sending && v.ctl.params().send) {
                v.queue.clear();
                v.queued_bytes = 0;
            }
            if v.queue.is_empty() {
                self.tokens = 0.0;
                return Vec::new();
            }
            let rate = v.ctl.bitrate() as f64 * 1.3 / 8.0 + 4_000.0;
            let cap = (rate * 0.03).max(2.0 * qt_core::call::VIDEO_MAX_PACKET as f64);
            self.tokens = (self.tokens + rate * dt).min(cap);
            let mut out = Vec::new();
            while let Some(len) = v.queue.front().map(|b| qt_core::call::video_packet_len(b.len())) {
                if (len as f64) > self.tokens {
                    break;
                }
                self.tokens -= len as f64;
                let body = v.queue.pop_front().expect("acabado de ver");
                v.queued_bytes -= len;
                // Cifrado so agora: o contador avanca so com o que sai.
                if let Some(p) = v.seal_now(&body, now) {
                    out.push(p);
                }
            }
            out
        }
    }

    /// A cada 100 ms: controlo de debito, relatorio para quem envia e pedidos de imagem
    /// completa. Devolve os pacotes a enviar.
    fn video_tick(shared: &Shared, direct: bool, audio_ref: &mut (u64, u64), now: Instant) -> Vec<Vec<u8>> {
        // Perdas do audio desde o ultimo relatorio (o audio tem sempre prioridade).
        let (lost, received) = shared
            .playback
            .lock()
            .map(|p| {
                let s = p.jitter.stats();
                (s.lost + s.gave_up, s.received)
            })
            .unwrap_or_default();
        let Ok(mut v) = shared.video.lock() else { return Vec::new() };
        v.ctl.set_path(direct);
        v.ctl.tick(now);
        let mut bodies = Vec::new();
        if v.receiving {
            let (dl, dr) = (lost.saturating_sub(audio_ref.0), received.saturating_sub(audio_ref.1));
            let pct = (dl * 100).checked_div(dl + dr).map_or(0, |p| p.min(100) as u8);
            let net = v.net;
            if let Some(fb) = v.rx.feedback(now, pct, net) {
                *audio_ref = (lost, received);
                bodies.push(encode_feedback(&fb));
            }
            let want = std::mem::take(&mut v.want_remote_keyframe);
            if v.rx.take_keyframe_request(now) || want {
                bodies.push(encode_keyframe_request());
            }
        } else {
            *audio_ref = (lost, received);
        }
        bodies.iter().filter_map(|b| v.seal_now(b, now)).collect()
    }

    /// Pacote de video do outro lado (pelo relay ou direto).
    fn on_video(shared: &Shared, data: &[u8], now: Instant) {
        let Ok(mut v) = shared.video.lock() else { return };
        let Some(session) = v.session.as_mut() else { return };
        let (counter, body) = match session.open(data) {
            Ok(x) => x,
            Err(CallError::Replay) => {
                v.stats.duplicates += 1;
                return;
            }
            Err(_) => {
                v.stats.invalid += 1;
                return;
            }
        };
        // Todos os pacotes contam para as perdas (o contador e um so para todos os tipos).
        if v.receiving {
            v.rx.on_packet(counter, data.len());
        }
        match parse_body(&body) {
            Some(Body::Frag(f)) => {
                // Sem ter aceitado: nem se le (consentimento de quem recebe).
                if v.receiving {
                    v.rx.on_fragment(f, now);
                    drop(v);
                    shared.video_cv.notify_all();
                }
            }
            Some(Body::Feedback(fb)) => {
                let stalled = v.stalled(fb.highest, now);
                v.stats.feedbacks += 1;
                v.stats.stalls += stalled as u64;
                if fb.expected > 0 {
                    v.stats.peer_loss_permille = 1000 - (fb.received as u64 * 1000 / fb.expected as u64) as u32;
                }
                v.ctl.on_feedback(&fb, stalled, now);
            }
            Some(Body::Keyframe) => {
                v.stats.keyframe_requests_received += 1;
                v.ctl.request_keyframe();
            }
            None => v.stats.invalid += 1,
        }
    }

    async fn open_udp(target: &str) -> Option<Udp> {
        let relay = canon(tokio::net::lookup_host(target).await.ok()?.next()?);
        // Dual (IPv6 + IPv4): o caminho direto pode ir por IPv6 mesmo com o relay so em IPv4.
        if let Some(sock) = dual_stack() {
            return Some(Udp { sock: Arc::new(sock), relay, dual: true, v6: true });
        }
        let local: SocketAddr = if relay.is_ipv4() { (Ipv4Addr::UNSPECIFIED, 0).into() } else { (Ipv6Addr::UNSPECIFIED, 0).into() };
        let sock = UdpSocket::bind(local).await.ok()?;
        Some(Udp { sock: Arc::new(sock), relay, dual: false, v6: relay.is_ipv6() })
    }

    fn dual_stack() -> Option<UdpSocket> {
        use socket2::{Domain, Protocol, Socket, Type};
        let s = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP)).ok()?;
        s.set_only_v6(false).ok()?;
        s.set_nonblocking(true).ok()?;
        s.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)).into()).ok()?;
        UdpSocket::from_std(s.into()).ok()
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
        if data.len() == RELAY_VIDEO_LEN && data[0] == RELAY_VIDEO_KIND {
            if let (Some(m), Ok(mut v)) = (parse_relay_video_notice(&shared.room, data), shared.video.lock()) {
                v.stats.relay_mode = m as u8;
                v.ctl.set_relay_mode(m);
            }
            return;
        }
        if is_video(data) {
            on_video(shared, data, Instant::now());
            return;
        }
        on_media(shared, cfg, rng, data);
    }

    /// Um pacote de audio (pelo relay ou direto). `true`: valido e entregue ao jitter buffer.
    fn on_media(shared: &Shared, cfg: &EngineConfig, rng: &mut Rng, data: &[u8]) -> bool {
        if cfg.simulated_loss_percent > 0 && rng.percent() < cfg.simulated_loss_percent {
            set_stats(shared, |s| s.simulated_drops += 1);
            return false;
        }
        let opened = shared.media.lock().ok().and_then(|mut m| m.as_mut().map(|m| m.open(data)));
        match opened {
            Some(Ok((counter, frame))) => {
                if let Ok(mut pb) = shared.playback.lock() {
                    pb.jitter.push(counter, frame, Instant::now());
                }
                set_stats(shared, |s| {
                    s.received += 1;
                    s.peer_present = true;
                });
                true
            }
            // O mesmo pacote pelos dois caminhos (ou repetido pela rede): a copia nao conta.
            Some(Err(CallError::Replay)) => {
                set_stats(shared, |s| s.duplicates += 1);
                false
            }
            _ => {
                set_stats(shared, |s| s.invalid += 1);
                false
            }
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

#[cfg(test)]
mod tests {
    use super::net::{canon, usable};
    use std::net::{IpAddr, SocketAddr};

    #[test]
    fn enderecos_que_valem_para_a_ligacao_direta() {
        let ok = |s: &str| usable(s.parse::<IpAddr>().unwrap(), false);
        assert!(ok("192.168.1.5") && ok("10.1.2.3") && ok("100.70.1.1") && ok("85.240.1.1"));
        assert!(ok("2a01:1::5") && ok("fd00::1"));
        assert!(!ok("127.0.0.1") && !ok("::1") && !ok("0.0.0.0") && !ok("::"));
        assert!(!ok("169.254.1.1") && !ok("fe80::1") && !ok("224.0.0.1") && !ok("ff02::1"));
        assert!(!ok("192.0.2.1") && !ok("2001:db8::1"), "documentacao");
        assert!(usable("127.0.0.1".parse().unwrap(), true), "testes");
        let m: SocketAddr = "[::ffff:1.2.3.4]:9".parse().unwrap();
        assert_eq!(canon(m), "1.2.3.4:9".parse().unwrap());
    }
}
