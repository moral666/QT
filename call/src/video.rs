//! Video de uma chamada (fase 5a): o que vai dentro dos pacotes de video, a remontagem das
//! imagens, a sincronizacao com o audio e o controlo do debito. Sem rede nem relogio
//! proprio (os testes simulam o tempo); a rede e o `engine`.
//!
//! A app (Kotlin) so trata da camara e do MediaCodec: entrega imagens H.264 ja
//! codificadas (`push`), aplica os parametros que daqui saem (`VideoParams`: debito,
//! resolucao, imagens/s, "imagem completa agora") e descodifica o que `pop` devolve.
//!
//! CONTEUDO DE UM PACOTE DE VIDEO (cifrado por `qt_core::call::VideoSession`):
//!   * pedaco de imagem: `[1 0][4 imagem][2 pedaco][2 pedacos][1 opcoes][4 ts][dados]`; opcoes:
//!     bit0 = imagem completa, bits 1-2 = quartos de volta (sentido dos ponteiros) para a mostrar
//!     direita (a camara grava sempre deitada; quem envia diz como o telemovel estava).
//!     `ts` = momento da captura na linha de tempo do AUDIO de quem envia (contador do
//!     audio x 20 ms): quem recebe mostra a imagem quando o audio que esta a tocar chega
//!     a esse momento (sincronizacao labial).
//!   * relatorio de quem recebe (a cada 250 ms): `[1 1][2 intervalo ms][4 recebidos][4 esperados]
//!     [2 atraso de fila ms][1 % perdido no audio][1 rede 0 ?, 1 Wi-Fi, 2 dados moveis][4 bit/s recebidos]
//!     [8 maior contador recebido + 1, 0 = nenhum]`. Recebidos e esperados contam TODOS os
//!     pacotes de video do outro lado (imagens, relatorios, pedidos): o contador e um so.
//!   * pedido de imagem completa: `[1 2]` (perdeu-se uma imagem: as seguintes dependem dela).
//!
//! DEBITO (quem envia): sobe 20%/s ate ao primeiro sinal de congestao e depois ~8%/s,
//! enquanto os relatorios vierem limpos; desce 15% (a partir do que chegou de facto) com
//! perdas, fila a crescer (atraso acima de 100 ms) ou perdas no audio (o audio tem sempre
//! prioridade), 30% quando nada esta a chegar. Nunca abaixo do minimo dos 240p: se a
//! congestao continuar ai durante 2 s, o video pausa. A resolucao segue o debito (escada de
//! 240p a 1080p), com tetos pelo caminho: pelo relay 360p (240p com o travao do mes, nada
//! com o teto da conta); em ligacao direta 1080p com Wi-Fi dos dois lados e 720p com dados
//! moveis. Abaixo do minimo dos 240p durante 2 s, o video pausa (a voz continua) e volta a
//! tentar quando o audio estiver limpo.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use qt_core::call::{RelayVideo, VIDEO_MAX_BODY};

pub const T_FRAG: u8 = 0;
pub const T_FEEDBACK: u8 = 1;
pub const T_KEYFRAME: u8 = 2;
/// Cabecalho de cada pedaco de imagem.
pub const FRAG_HDR: usize = 14;
/// Dados de imagem em cada pacote cheio.
pub const FRAG_DATA: usize = VIDEO_MAX_BODY - FRAG_HDR;
/// Maior imagem aceite (uma imagem completa em 1080p fica muito abaixo disto).
pub const MAX_FRAME_BYTES: usize = 1 << 20;
const MAX_FRAGS: usize = MAX_FRAME_BYTES.div_ceil(FRAG_DATA);
const FEEDBACK_LEN: usize = 27;
/// Relatorios de quem recebe.
pub const FEEDBACK_EVERY: Duration = Duration::from_millis(250);

/// Rede do telemovel (a app diz qual; desconhecida conta como dados moveis).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Net {
    #[default]
    Unknown,
    Wifi,
    Mobile,
}

impl Net {
    fn to_byte(self) -> u8 {
        match self {
            Net::Unknown => 0,
            Net::Wifi => 1,
            Net::Mobile => 2,
        }
    }
    fn from_byte(b: u8) -> Net {
        match b {
            1 => Net::Wifi,
            2 => Net::Mobile,
            _ => Net::Unknown,
        }
    }
}

// ------------------------------------------------------------------ formato

/// Um pedaco de imagem recebido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frag {
    pub frame_id: u32,
    pub index: u16,
    pub count: u16,
    pub keyframe: bool,
    /// Quartos de volta (0 a 3) para mostrar a imagem direita.
    pub rotation: u8,
    pub ts_ms: u32,
    pub data: Vec<u8>,
}

/// Relatorio de quem recebe (ver o inicio do ficheiro).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Feedback {
    pub interval_ms: u16,
    pub received: u32,
    pub expected: u32,
    pub queue_delay_ms: u16,
    pub audio_loss_pct: u8,
    pub net: Net,
    pub recv_bps: u32,
    /// Maior contador de video recebido do outro lado (qualquer tipo de pacote).
    pub highest: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Frag(Frag),
    Feedback(Feedback),
    Keyframe,
}

/// Parte uma imagem em pedacos (um por pacote). Os pacotes cheios tem 1168 bytes de dados.
/// `rotation`: quartos de volta para a mostrar direita.
pub fn fragment(frame_id: u32, keyframe: bool, rotation: u8, ts_ms: u32, data: &[u8]) -> Vec<Vec<u8>> {
    let count = data.len().div_ceil(FRAG_DATA).max(1);
    (0..count)
        .map(|i| {
            let chunk = &data[(i * FRAG_DATA).min(data.len())..((i + 1) * FRAG_DATA).min(data.len())];
            let mut b = Vec::with_capacity(FRAG_HDR + chunk.len());
            b.push(T_FRAG);
            b.extend_from_slice(&frame_id.to_be_bytes());
            b.extend_from_slice(&(i as u16).to_be_bytes());
            b.extend_from_slice(&(count as u16).to_be_bytes());
            b.push(keyframe as u8 | (rotation & 3) << 1);
            b.extend_from_slice(&ts_ms.to_be_bytes());
            b.extend_from_slice(chunk);
            b
        })
        .collect()
}

pub fn encode_feedback(f: &Feedback) -> Vec<u8> {
    let mut b = Vec::with_capacity(FEEDBACK_LEN);
    b.push(T_FEEDBACK);
    b.extend_from_slice(&f.interval_ms.to_be_bytes());
    b.extend_from_slice(&f.received.to_be_bytes());
    b.extend_from_slice(&f.expected.to_be_bytes());
    b.extend_from_slice(&f.queue_delay_ms.to_be_bytes());
    b.push(f.audio_loss_pct);
    b.push(f.net.to_byte());
    b.extend_from_slice(&f.recv_bps.to_be_bytes());
    b.extend_from_slice(&f.highest.map_or(0, |h| h.saturating_add(1)).to_be_bytes());
    b
}

pub fn encode_keyframe_request() -> Vec<u8> {
    vec![T_KEYFRAME]
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Le o conteudo de um pacote de video ja decifrado. `None`: invalido (descartar).
pub fn parse_body(b: &[u8]) -> Option<Body> {
    match *b.first()? {
        T_FRAG if b.len() > FRAG_HDR => {
            let (index, count) = (be16(&b[5..7]), be16(&b[7..9]));
            if count == 0 || index >= count || count as usize > MAX_FRAGS || b[9] & !0x07 != 0 {
                return None;
            }
            Some(Body::Frag(Frag {
                frame_id: be32(&b[1..5]),
                index,
                count,
                keyframe: b[9] & 1 == 1,
                rotation: (b[9] >> 1) & 3,
                ts_ms: be32(&b[10..14]),
                data: b[FRAG_HDR..].to_vec(),
            }))
        }
        T_FEEDBACK if b.len() == FEEDBACK_LEN => Some(Body::Feedback(Feedback {
            interval_ms: be16(&b[1..3]),
            received: be32(&b[3..7]),
            expected: be32(&b[7..11]),
            queue_delay_ms: be16(&b[11..13]),
            audio_loss_pct: b[13].min(100),
            net: Net::from_byte(b[14]),
            recv_bps: be32(&b[15..19]),
            highest: u64::from_be_bytes(b[19..27].try_into().ok()?).checked_sub(1),
        })),
        T_KEYFRAME if b.len() == 1 => Some(Body::Keyframe),
        _ => None,
    }
}

// ------------------------------------------------------------------ rececao

/// Uma imagem pronta a descodificar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedFrame {
    pub data: Vec<u8>,
    pub keyframe: bool,
    /// Quartos de volta (0 a 3) para mostrar a imagem direita.
    pub rotation: u8,
    pub ts_ms: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoRecvStats {
    pub packets: u64,
    pub bytes: u64,
    pub frames: u64,
    pub keyframes: u64,
    /// Imagens que nunca ficaram completas a tempo (e as que dependiam delas).
    pub lost_frames: u64,
    pub keyframe_requests: u64,
    /// Pacotes que deviam ter chegado (pelos contadores) e os que chegaram, somados nos
    /// relatorios: a perda medida e `1 - reported / expected`.
    pub expected: u64,
    pub reported: u64,
    /// Video atras do audio quando a ultima imagem saiu (ms; negativo = a frente).
    pub av_lag_ms: i32,
    /// Atraso de fila medido (ms), o do ultimo relatorio.
    pub queue_delay_ms: u16,
}

struct Partial {
    count: u16,
    parts: Vec<Option<Vec<u8>>>,
    got: u16,
    bytes: usize,
    keyframe: bool,
    rotation: u8,
    ts_ms: u32,
    first_at: Instant,
    done_at: Option<Instant>,
}

impl Partial {
    fn complete(&self) -> bool {
        self.done_at.is_some()
    }
}

/// Espera por um pedaco em falta enquanto houver imagens completas a seguir: cobre a copia
/// que vem pelo relay (+50 a 150 ms) quando o direto perde um pacote.
const LOSS_WAIT: Duration = Duration::from_millis(150);
/// Imagem incompleta ha mais disto: perdida.
const FRAME_TIMEOUT: Duration = Duration::from_millis(400);
/// A imagem sai quando o audio a tocar chega ao seu `ts` (menos isto: descodificar e mostrar
/// tambem demora)...
const SYNC_LEAD_MS: i64 = 30;
/// ... mas nunca espera mais do que isto pelo audio (audio parado ou muito atrasado).
const MAX_SYNC_WAIT: Duration = Duration::from_millis(400);
/// Limites de memoria da remontagem.
const MAX_PARTIALS: usize = 90;
const MAX_PENDING_BYTES: usize = 6 << 20;
/// Pedidos de imagem completa: o primeiro logo, depois cada vez mais espacados.
const KF_FIRST: Duration = Duration::from_millis(250);
const KF_MAX: Duration = Duration::from_secs(2);
/// Janela para o atraso minimo (base) das chegadas.
const DELAY_WINDOW: Duration = Duration::from_secs(10);

/// Remontagem, espera pelos atrasados, sincronizacao com o audio e relatorios.
pub struct VideoReceiver {
    frames: BTreeMap<u32, Partial>,
    next: Option<u32>,
    need_key: bool,
    pending_bytes: usize,
    kf_last: Option<Instant>,
    kf_backoff: Duration,
    epoch: Instant,
    // relatorios
    highest: Option<u64>,
    interval_highest: Option<u64>,
    interval_received: u32,
    interval_bytes: u64,
    interval_start: Instant,
    transits: VecDeque<(Instant, i64)>,
    interval_transit: (i64, u32),
    pub stats: VideoRecvStats,
}

impl VideoReceiver {
    pub fn new(now: Instant) -> Self {
        VideoReceiver {
            frames: BTreeMap::new(),
            next: None,
            need_key: true,
            pending_bytes: 0,
            kf_last: None,
            kf_backoff: KF_FIRST,
            epoch: now,
            highest: None,
            interval_highest: None,
            interval_received: 0,
            interval_bytes: 0,
            interval_start: now,
            transits: VecDeque::new(),
            interval_transit: (0, 0),
            stats: VideoRecvStats::default(),
        }
    }

    /// Recomeca (o outro lado voltou a ligar a camara): espera por uma imagem completa.
    pub fn reset(&mut self, now: Instant) {
        let stats = std::mem::take(&mut self.stats);
        *self = VideoReceiver::new(now);
        self.stats = stats;
    }

    /// QUALQUER pacote de video valido do outro lado (imagem, relatorio ou pedido): o
    /// contador e um so para todos, e contar so as imagens fazia dos outros "perdas".
    pub fn on_packet(&mut self, counter: u64, len: usize) {
        self.stats.packets += 1;
        self.stats.bytes += len as u64;
        self.interval_received += 1;
        self.interval_bytes += len as u64;
        if self.highest.is_none_or(|h| counter > h) {
            self.highest = Some(counter);
        }
    }

    pub fn on_fragment(&mut self, f: Frag, now: Instant) {
        if self.next.is_some_and(|n| f.frame_id < n) {
            return; // ja passou
        }
        // Atraso de fila: a chegada do primeiro pedaco de cada imagem (nao completa: essas
        // demoram mais a sair do ritmador) contra o momento da captura.
        if f.index == 0 && !f.keyframe {
            let transit = now.duration_since(self.epoch).as_millis() as i64 - f.ts_ms as i64;
            self.transits.push_back((now, transit));
            while self.transits.front().is_some_and(|(t, _)| now.duration_since(*t) > DELAY_WINDOW) {
                self.transits.pop_front();
            }
            self.interval_transit.0 += transit;
            self.interval_transit.1 += 1;
        }
        let p = self.frames.entry(f.frame_id).or_insert_with(|| Partial {
            count: f.count,
            parts: vec![None; f.count as usize],
            got: 0,
            bytes: 0,
            keyframe: f.keyframe,
            rotation: f.rotation,
            ts_ms: f.ts_ms,
            first_at: now,
            done_at: None,
        });
        if p.count != f.count || p.parts[f.index as usize].is_some() {
            return;
        }
        if p.bytes + f.data.len() > MAX_FRAME_BYTES {
            return;
        }
        p.bytes += f.data.len();
        self.pending_bytes += f.data.len();
        p.parts[f.index as usize] = Some(f.data);
        p.got += 1;
        if p.got == p.count {
            p.done_at = Some(now);
        }
        self.prune();
    }

    fn remove_frame(&mut self, id: u32) -> Option<Partial> {
        let p = self.frames.remove(&id)?;
        self.pending_bytes -= p.bytes;
        Some(p)
    }

    fn prune(&mut self) {
        while self.frames.len() > MAX_PARTIALS || self.pending_bytes > MAX_PENDING_BYTES {
            let Some(&oldest) = self.frames.keys().next() else { break };
            self.remove_frame(oldest);
            self.stats.lost_frames += 1;
            self.need_key = true;
        }
    }

    /// Ha uma imagem completa a caminho (nao vale a pena pedir outra)?
    fn keyframe_in_flight(&self, now: Instant) -> bool {
        self.frames.values().any(|p| p.keyframe && now.duration_since(p.first_at) < FRAME_TIMEOUT)
    }

    /// A proxima imagem para descodificar, ou `None` (ainda nao). `audio_ts`: o momento (na
    /// linha de tempo de quem envia) do audio que esta a tocar agora, se houver audio.
    pub fn pop(&mut self, now: Instant, audio_ts: Option<u32>) -> Option<ReceivedFrame> {
        loop {
            if self.need_key {
                // So uma imagem completa serve para recomecar: salta para a mais recente.
                let key = self.frames.iter().rev().find(|(_, p)| p.keyframe && p.complete()).map(|(id, _)| *id)?;
                let older: Vec<u32> = self.frames.range(..key).map(|(id, _)| *id).collect();
                for id in older {
                    self.remove_frame(id);
                }
                self.next = Some(key);
                self.need_key = false;
                self.kf_backoff = KF_FIRST;
            }
            let n = self.next?;
            let ready = self.frames.get(&n).map(|p| (p.complete(), p.ts_ms, p.done_at, p.first_at));
            match ready {
                Some((true, ts, done_at, _)) => {
                    if let (Some(a), Some(d)) = (audio_ts, done_at) {
                        let early = ts as i64 - a as i64 - SYNC_LEAD_MS;
                        if early > 0 && now.duration_since(d) < MAX_SYNC_WAIT {
                            return None; // o audio ainda nao chegou la
                        }
                    }
                    let p = self.remove_frame(n).expect("acabada de ver");
                    self.next = Some(n.wrapping_add(1));
                    self.stats.frames += 1;
                    if p.keyframe {
                        self.stats.keyframes += 1;
                    }
                    if let Some(a) = audio_ts {
                        self.stats.av_lag_ms = (a as i64 - p.ts_ms as i64).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
                    }
                    let mut data = Vec::with_capacity(p.bytes);
                    for part in p.parts.into_iter().flatten() {
                        data.extend_from_slice(&part);
                    }
                    return Some(ReceivedFrame { data, keyframe: p.keyframe, rotation: p.rotation, ts_ms: p.ts_ms });
                }
                other => {
                    // Falta (ou esta incompleta). Perdida se ja houver imagens completas a
                    // seguir a espera ha `LOSS_WAIT`, ou se esta incompleta ha muito.
                    let later_waiting = self
                        .frames
                        .range(n.wrapping_add(1)..)
                        .filter_map(|(_, p)| p.done_at)
                        .min()
                        .is_some_and(|d| now.duration_since(d) >= LOSS_WAIT);
                    let stale = other.is_some_and(|(_, _, _, first)| now.duration_since(first) >= FRAME_TIMEOUT);
                    if !(later_waiting || stale) {
                        return None;
                    }
                    self.remove_frame(n);
                    self.stats.lost_frames += 1;
                    // As seguintes dependem desta: so uma imagem completa recomeca.
                    self.need_key = true;
                }
            }
        }
    }

    /// Pedir ao outro lado uma imagem completa agora? (perdeu-se uma e nenhuma vem a caminho)
    pub fn take_keyframe_request(&mut self, now: Instant) -> bool {
        if !self.need_key || self.keyframe_in_flight(now) {
            return false;
        }
        // A primeira imagem ainda nao chegou: da-lhe um momento antes de pedir.
        if self.next.is_none() && self.stats.packets == 0 {
            return false;
        }
        if self.kf_last.is_some_and(|t| now.duration_since(t) < self.kf_backoff) {
            return false;
        }
        if self.kf_last.is_some() {
            self.kf_backoff = (self.kf_backoff * 2).min(KF_MAX);
        }
        self.kf_last = Some(now);
        self.stats.keyframe_requests += 1;
        true
    }

    /// Relatorio para quem envia, a cada `FEEDBACK_EVERY`.
    pub fn feedback(&mut self, now: Instant, audio_loss_pct: u8, net: Net) -> Option<Feedback> {
        let dt = now.duration_since(self.interval_start);
        if dt < FEEDBACK_EVERY {
            return None;
        }
        let expected = match (self.highest, self.interval_highest) {
            (Some(h), Some(prev)) => (h - prev) as u32,
            (Some(_), None) => self.interval_received,
            _ => 0,
        };
        let base = self.transits.iter().map(|(_, t)| *t).min();
        let queue = match (base, self.interval_transit) {
            (Some(b), (sum, n)) if n > 0 => (sum / n as i64 - b).clamp(0, u16::MAX as i64) as u16,
            _ => 0,
        };
        let ms = dt.as_millis().max(1) as u64;
        let f = Feedback {
            interval_ms: ms.min(u16::MAX as u64) as u16,
            received: self.interval_received,
            expected: expected.max(self.interval_received),
            queue_delay_ms: queue,
            audio_loss_pct,
            net,
            recv_bps: (self.interval_bytes * 8 * 1000 / ms).min(u32::MAX as u64) as u32,
            highest: self.highest,
        };
        self.stats.queue_delay_ms = queue;
        self.stats.expected += f.expected as u64;
        self.stats.reported += f.received as u64;
        self.interval_start = now;
        self.interval_highest = self.highest;
        self.interval_received = 0;
        self.interval_bytes = 0;
        self.interval_transit = (0, 0);
        Some(f)
    }
}

// ------------------------------------------------------------------ debito

/// Um degrau da escada de resolucoes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rung {
    pub width: u16,
    pub height: u16,
    pub fps: u8,
    /// Debito minimo para este degrau (abaixo disto, desce).
    pub min_bps: u32,
    /// Debito maximo que vale a pena neste degrau.
    pub max_bps: u32,
}

pub const LADDER: [Rung; 5] = [
    Rung { width: 426, height: 240, fps: 15, min_bps: 150_000, max_bps: 350_000 },
    Rung { width: 640, height: 360, fps: 30, min_bps: 350_000, max_bps: 800_000 },
    Rung { width: 854, height: 480, fps: 30, min_bps: 700_000, max_bps: 1_400_000 },
    Rung { width: 1280, height: 720, fps: 30, min_bps: 1_200_000, max_bps: 2_500_000 },
    Rung { width: 1920, height: 1080, fps: 30, min_bps: 2_200_000, max_bps: 4_000_000 },
];
const R240: usize = 0;
const R360: usize = 1;
const R720: usize = 3;
const R1080: usize = 4;
/// Pelo relay: ate 360p a 800 kbit/s; com o travao do mes, 240p a 300 kbit/s.
pub const RELAY_MAX_BPS: u32 = 800_000;
pub const RELAY_CAP240_BPS: u32 = 300_000;
const START_RELAY_BPS: f64 = 500_000.0;
const START_DIRECT_BPS: f64 = 1_200_000.0;
/// Nunca abaixo disto sem pausar (minimo dos 240p).
const FLOOR_BPS: f64 = 150_000.0;
/// Subida por segundo: no arranque (ate ao primeiro sinal de congestao) e depois.
const SLOW_START_GROWTH: f64 = 0.20;
const GROWTH: f64 = 0.08;

/// Porque o video esta em pausa (a voz continua).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pause {
    /// Rede fraca (ou o relay nao deixa passar video: versao antiga).
    WeakNetwork,
    /// Pelo relay, sem video: teto da conta. Em ligacao direta continua.
    RelayLimit,
}

/// O que a app aplica no codificador.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoParams {
    /// Enviar video agora (camara ligada, o outro aceitou, sem pausa).
    pub send: bool,
    pub bitrate_bps: u32,
    pub width: u16,
    pub height: u16,
    pub fps: u8,
    pub paused: Option<Pause>,
    /// A qualidade esta limitada por ir pelo relay (ligacao direta daria mais).
    pub limited_by_relay: bool,
    /// Muda quando e preciso uma imagem completa ja (o outro perdeu uma, recomecou, ...).
    pub keyframe_seq: u32,
}

/// Sem relatorios ha tanto tempo, a enviar: rede a falhar.
const FEEDBACK_STALE: Duration = Duration::from_millis(1500);
/// A enviar ha tanto tempo e NUNCA chegou um relatorio: o relay nao deixa passar video
/// (versao antiga) ou o caminho nao serve: pausa.
const NO_FEEDBACK_PAUSE: Duration = Duration::from_secs(4);
const DECREASE_EVERY: Duration = Duration::from_millis(500);
const INCREASE_AFTER_DECREASE: Duration = Duration::from_secs(1);
const UPGRADE_HOLD: Duration = Duration::from_secs(2);
const WEAK_PAUSE_AFTER: Duration = Duration::from_secs(2);
const RESUME_AFTER: Duration = Duration::from_secs(4);
const RESUME_BPS: f64 = 200_000.0;

pub struct RateController {
    sending: bool,
    sending_since: Option<Instant>,
    direct: bool,
    relay_mode: RelayVideo,
    local_net: Net,
    peer_net: Net,
    bitrate: f64,
    rung: usize,
    upgrade_since: Option<Instant>,
    last_decrease: Option<Instant>,
    last_feedback: Option<Instant>,
    /// Congestao com o debito ja no minimo, desde quando (2 s seguidos: pausa).
    weak_since: Option<Instant>,
    paused_weak_since: Option<Instant>,
    clean_reports: u32,
    keyframe_seq: u32,
    last_stale_cut: Option<Instant>,
    slow_start: bool,
}

impl Default for RateController {
    fn default() -> Self {
        Self::new()
    }
}

impl RateController {
    pub fn new() -> Self {
        RateController {
            sending: false,
            sending_since: None,
            direct: false,
            relay_mode: RelayVideo::Full,
            local_net: Net::Unknown,
            peer_net: Net::Unknown,
            bitrate: START_RELAY_BPS,
            rung: R360,
            upgrade_since: None,
            last_decrease: None,
            last_feedback: None,
            weak_since: None,
            paused_weak_since: None,
            clean_reports: 0,
            keyframe_seq: 0,
            last_stale_cut: None,
            slow_start: true,
        }
    }

    /// Pelo relay e sem video la (teto da conta): pausa, sem mexer no debito.
    fn relay_denied(&self) -> bool {
        !self.direct && self.relay_mode == RelayVideo::Denied
    }

    /// Teto do caminho atual: (degrau maximo, debito maximo).
    fn caps(&self) -> (usize, u32) {
        if !self.direct {
            return match self.relay_mode {
                RelayVideo::Full => (R360, RELAY_MAX_BPS),
                RelayVideo::Cap240 => (R240, RELAY_CAP240_BPS),
                RelayVideo::Denied => (R240, 0),
            };
        }
        let wifi = self.local_net == Net::Wifi && self.peer_net != Net::Mobile;
        if wifi {
            (R1080, LADDER[R1080].max_bps)
        } else {
            (R720, LADDER[R720].max_bps)
        }
    }

    fn clamp(&mut self) {
        let (_, max) = self.caps();
        if max == 0 {
            return; // sem video por este caminho: o debito fica como estava
        }
        self.bitrate = self.bitrate.min(max as f64).max(FLOOR_BPS.min(max as f64));
    }

    /// Camara ligada e o outro aceitou (ou deixou de ser assim).
    pub fn set_sending(&mut self, on: bool, now: Instant) {
        if on == self.sending {
            return;
        }
        self.sending = on;
        if on {
            self.sending_since = Some(now);
            self.last_feedback = None;
            self.weak_since = None;
            self.paused_weak_since = None;
            self.bitrate = if self.direct { START_DIRECT_BPS } else { START_RELAY_BPS };
            self.slow_start = true;
            self.clamp();
            self.rung = self.best_rung(false);
            self.keyframe_seq = self.keyframe_seq.wrapping_add(1);
        } else {
            self.sending_since = None;
        }
    }

    /// O caminho do audio (direto ou relay). Ao passar para o relay, desce logo ao teto dele;
    /// ao passar para direto, salta para o arranque do direto (e volta a subir depressa).
    pub fn set_path(&mut self, direct: bool) {
        if direct == self.direct {
            return;
        }
        let was_denied = self.relay_denied();
        self.direct = direct;
        if direct {
            if self.bitrate < START_DIRECT_BPS {
                self.bitrate = START_DIRECT_BPS;
                self.slow_start = true;
            }
            if was_denied {
                self.weak_since = None;
                self.keyframe_seq = self.keyframe_seq.wrapping_add(1);
            }
        }
        self.clamp();
        let best = self.best_rung(false);
        // Para direto: o degrau sobe ja (o caminho melhorou de uma vez). Para o relay: desce.
        if direct || best < self.rung {
            self.rung = best;
            self.upgrade_since = None;
        }
    }

    pub fn set_relay_mode(&mut self, m: RelayVideo) {
        if m != self.relay_mode {
            let was_denied = self.relay_mode == RelayVideo::Denied;
            self.relay_mode = m;
            if was_denied && !self.direct {
                self.bitrate = START_RELAY_BPS;
                self.slow_start = true;
                self.weak_since = None;
                self.keyframe_seq = self.keyframe_seq.wrapping_add(1);
            }
            self.clamp();
            self.rung = if was_denied { self.best_rung(false) } else { self.rung.min(self.caps().0) };
        }
    }

    pub fn set_local_net(&mut self, n: Net) {
        self.local_net = n;
        self.clamp();
        self.rung = self.rung.min(self.caps().0);
    }

    /// O outro lado pediu uma imagem completa (ou recomecou).
    pub fn request_keyframe(&mut self) {
        self.keyframe_seq = self.keyframe_seq.wrapping_add(1);
    }

    fn best_rung(&self, upgrading: bool) -> usize {
        let (max_rung, _) = self.caps();
        let mut r = 0;
        for (i, rung) in LADDER.iter().enumerate().take(max_rung + 1) {
            let need = if upgrading && i > self.rung { rung.min_bps as f64 * 1.15 } else { rung.min_bps as f64 };
            if self.bitrate >= need {
                r = i;
            }
        }
        r
    }

    fn weak_paused(&self) -> bool {
        self.paused_weak_since.is_some()
    }

    /// Desce (nunca abaixo do minimo). Ja no minimo, conta como "rede fraca".
    fn decrease(&mut self, anchor: Option<u32>, factor: f64, now: Instant) {
        self.last_decrease = Some(now);
        self.slow_start = false;
        if self.bitrate <= FLOOR_BPS * 1.001 {
            self.weak_since.get_or_insert(now);
            return;
        }
        let base = match anchor.filter(|a| *a > 0) {
            Some(a) => self.bitrate.min(a as f64),
            None => self.bitrate,
        };
        self.bitrate = (base * factor).max(FLOOR_BPS);
    }

    /// Um relatorio do outro lado. `stalled`: ha pacotes enviados ha mais de 800 ms que o
    /// relatorio ainda nao viu (nada esta a chegar). Um relatorio vazio logo depois de
    /// comecar ou de retomar nao e perda: os pacotes ainda vao a caminho.
    pub fn on_feedback(&mut self, fb: &Feedback, stalled: bool, now: Instant) {
        self.last_feedback = Some(now);
        self.peer_net = fb.net;
        self.clamp();
        if self.weak_paused() {
            // Em pausa nao chega video: so o audio diz se a rede melhorou.
            self.clean_reports = if fb.audio_loss_pct >= 1 { 0 } else { self.clean_reports + 1 };
            return;
        }
        let loss = if stalled {
            1.0
        } else if fb.expected > 0 {
            1.0 - fb.received as f64 / fb.expected as f64
        } else {
            0.0
        };
        if self.relay_denied() {
            return;
        }
        // Sem pacotes no intervalo (e sem nada parado) o relatorio nao diz nada do video: nao
        // sobe nem desce por ele (o audio continua a contar).
        let informative = fb.expected > 0 || stalled;
        let congested = loss > 0.10 || fb.queue_delay_ms > 100 || fb.audio_loss_pct >= 3;
        let mild = loss > 0.03 || fb.queue_delay_ms > 50 || fb.audio_loss_pct >= 1 || !informative;
        if congested {
            if self.last_decrease.is_none_or(|t| now.duration_since(t) >= DECREASE_EVERY) {
                // Nada a chegar: corta mais.
                let factor = if stalled { 0.7 } else { 0.85 };
                self.decrease(Some(fb.recv_bps), factor, now);
            }
        } else if !mild {
            self.weak_since = None;
            if self.last_decrease.is_none_or(|t| now.duration_since(t) >= INCREASE_AFTER_DECREASE) {
                let dt = (fb.interval_ms as f64 / 1000.0).min(0.5);
                let growth = if self.slow_start { SLOW_START_GROWTH } else { GROWTH };
                self.bitrate *= 1.0 + growth * dt;
            }
        }
        self.clamp();
    }

    /// A cada ~100 ms.
    pub fn tick(&mut self, now: Instant) {
        if !self.sending || self.relay_denied() {
            return;
        }
        // Relatorios que deixaram de chegar: corta.
        let since = self.sending_since.unwrap_or(now);
        match self.last_feedback {
            None if now.duration_since(since) >= NO_FEEDBACK_PAUSE && !self.weak_paused() => {
                self.paused_weak_since = Some(now);
            }
            Some(t)
                if now.duration_since(t) >= FEEDBACK_STALE
                    && !self.weak_paused()
                    && self.last_stale_cut.is_none_or(|c| now.duration_since(c) >= DECREASE_EVERY) =>
            {
                self.decrease(None, 0.7, now);
                self.last_stale_cut = Some(now);
            }
            _ => {}
        }
        // Pausa por rede fraca e regresso.
        if let Some(p) = self.paused_weak_since {
            let clean = self.clean_reports >= 4 && self.last_feedback.is_some_and(|t| now.duration_since(t) < FEEDBACK_STALE);
            if now.duration_since(p) >= RESUME_AFTER && clean {
                self.paused_weak_since = None;
                self.weak_since = None;
                self.bitrate = RESUME_BPS;
                self.slow_start = false;
                self.clamp();
                self.rung = self.best_rung(false);
                self.sending_since = Some(now);
                self.last_decrease = Some(now);
                self.keyframe_seq = self.keyframe_seq.wrapping_add(1);
            }
            return;
        }
        // Congestao ja no minimo durante 2 s: pausa.
        if self.weak_since.is_some_and(|w| now.duration_since(w) >= WEAK_PAUSE_AFTER) {
            self.paused_weak_since = Some(now);
            self.clean_reports = 0;
            return;
        }
        // Degrau: desce logo; sobe so depois de 2 s com debito para isso.
        let down_to = self.best_rung(false);
        if self.bitrate < LADDER[self.rung].min_bps as f64 * 0.9 || self.rung > self.caps().0 {
            self.rung = down_to.min(self.caps().0);
            self.upgrade_since = None;
        } else {
            let up = self.best_rung(true);
            if up > self.rung {
                let s = *self.upgrade_since.get_or_insert(now);
                if now.duration_since(s) >= UPGRADE_HOLD {
                    self.rung = up;
                    self.upgrade_since = None;
                }
            } else {
                self.upgrade_since = None;
            }
        }
    }

    pub fn params(&self) -> VideoParams {
        let (max_rung, max_bps) = self.caps();
        let paused = if self.relay_denied() {
            Some(Pause::RelayLimit)
        } else if self.weak_paused() {
            Some(Pause::WeakNetwork)
        } else {
            None
        };
        let rung = LADDER[self.rung.min(max_rung)];
        let bitrate = if max_bps == 0 { FLOOR_BPS as u32 } else { (self.bitrate as u32).clamp((FLOOR_BPS as u32).min(max_bps), max_bps) };
        VideoParams {
            send: self.sending && paused.is_none(),
            bitrate_bps: bitrate.min(rung.max_bps),
            width: rung.width,
            height: rung.height,
            fps: rung.fps,
            paused,
            limited_by_relay: !self.direct,
            keyframe_seq: self.keyframe_seq,
        }
    }

    /// Debito atual pedido (para o ritmador).
    pub fn bitrate(&self) -> u32 {
        self.params().bitrate_bps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    fn frags(id: u32, key: bool, ts: u32, len: usize) -> Vec<Frag> {
        let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8 ^ id as u8).collect();
        fragment(id, key, 0, ts, &data)
            .into_iter()
            .map(|b| match parse_body(&b) {
                Some(Body::Frag(f)) => f,
                _ => panic!(),
            })
            .collect()
    }

    #[test]
    fn formato_ida_e_volta() {
        let data = vec![7u8; FRAG_DATA * 2 + 5];
        let fs = fragment(3, true, 3, 1234, &data);
        assert_eq!(fs.len(), 3);
        assert!(fs.iter().all(|b| b.len() <= VIDEO_MAX_BODY));
        assert_eq!(fs[0].len(), VIDEO_MAX_BODY, "pacotes cheios");
        let Some(Body::Frag(f)) = parse_body(&fs[2]) else { panic!() };
        assert_eq!((f.frame_id, f.index, f.count, f.keyframe, f.rotation, f.ts_ms, f.data.len()), (3, 2, 3, true, 3, 1234, 5));
        let Some(Body::Frag(g)) = parse_body(&fragment(4, false, 1, 0, &[1, 2])[0]) else { panic!() };
        assert_eq!((g.keyframe, g.rotation), (false, 1));
        let fb = Feedback { interval_ms: 250, received: 9, expected: 10, queue_delay_ms: 40, audio_loss_pct: 2, net: Net::Wifi, recv_bps: 512_000, highest: Some(0) };
        assert_eq!(parse_body(&encode_feedback(&fb)), Some(Body::Feedback(fb)));
        let none = Feedback { highest: None, ..fb };
        assert_eq!(parse_body(&encode_feedback(&none)), Some(Body::Feedback(none)));
        assert_eq!(parse_body(&encode_keyframe_request()), Some(Body::Keyframe));
        // Lixo: pedaco fora do total, total zero, tamanhos errados.
        let mut bad = fs[0].clone();
        bad[9] = 0x08;
        assert_eq!(parse_body(&bad), None, "opcoes desconhecidas");
        let mut bad = fs[0].clone();
        bad[5..7].copy_from_slice(&3u16.to_be_bytes());
        assert_eq!(parse_body(&bad), None);
        bad[7..9].copy_from_slice(&0u16.to_be_bytes());
        assert_eq!(parse_body(&bad), None);
        assert_eq!(parse_body(&[T_FEEDBACK, 1, 2]), None);
        assert_eq!(parse_body(&[9]), None);
    }

    #[test]
    fn remonta_por_ordem_e_comeca_numa_imagem_completa() {
        let t0 = Instant::now();
        let mut r = VideoReceiver::new(t0);
        // Uma imagem normal antes da primeira completa: nao serve para comecar.
        for f in frags(0, false, 0, 3000) {
            r.on_fragment(f, at(t0, 0));
        }
        assert_eq!(r.pop(at(t0, 1), None), None);
        // Pedacos da completa fora de ordem.
        let mut k = frags(1, true, 33, 5000);
        k.reverse();
        for f in k {
            r.on_fragment(f, at(t0, 10));
        }
        for f in frags(2, false, 66, 900) {
            r.on_fragment(f, at(t0, 20));
        }
        let a = r.pop(at(t0, 30), None).unwrap();
        assert!(a.keyframe && a.data.len() == 5000 && a.ts_ms == 33);
        assert_eq!(a.data, frags(1, true, 33, 5000).into_iter().flat_map(|f| f.data).collect::<Vec<_>>());
        assert_eq!(r.pop(at(t0, 30), None).map(|f| f.ts_ms), Some(66));
        assert_eq!(r.pop(at(t0, 30), None), None);
    }

    #[test]
    fn perda_espera_pela_copia_e_depois_pede_imagem_completa() {
        let t0 = Instant::now();
        let mut r = VideoReceiver::new(t0);
        for f in frags(0, true, 0, 2000) {
            r.on_fragment(f, at(t0, 0));
        }
        assert!(r.pop(at(t0, 0), None).is_some());
        // Imagem 1: falta um pedaco; a 2 chega completa.
        let f1 = frags(1, false, 33, 3000);
        r.on_fragment(f1[0].clone(), at(t0, 33));
        for f in frags(2, false, 66, 800) {
            r.on_fragment(f, at(t0, 66));
        }
        // A copia pelo relay chega 100 ms depois: ainda a tempo.
        assert_eq!(r.pop(at(t0, 120), None), None);
        r.on_fragment(f1[1].clone(), at(t0, 160));
        r.on_fragment(f1[2].clone(), at(t0, 160));
        assert_eq!(r.pop(at(t0, 161), None).map(|f| f.ts_ms), Some(33));
        assert_eq!(r.pop(at(t0, 161), None).map(|f| f.ts_ms), Some(66));
        // Imagem 3 perdida de vez: a 4 espera 150 ms, depois perde-se tudo ate uma completa.
        for f in frags(4, false, 132, 800) {
            r.on_fragment(f, at(t0, 132));
        }
        assert_eq!(r.pop(at(t0, 200), None), None);
        assert_eq!(r.pop(at(t0, 290), None), None);
        assert_eq!(r.stats.lost_frames, 1);
        assert!(r.take_keyframe_request(at(t0, 290)));
        assert!(!r.take_keyframe_request(at(t0, 300)), "um pedido de cada vez");
        assert!(r.take_keyframe_request(at(t0, 560)), "repete se nao vier");
        assert!(!r.take_keyframe_request(at(t0, 700)), "cada vez mais espacado");
        for f in frags(9, true, 300, 4000) {
            r.on_fragment(f, at(t0, 600));
        }
        let k = r.pop(at(t0, 601), None).unwrap();
        assert!(k.keyframe && k.ts_ms == 300);
        assert!(!r.take_keyframe_request(at(t0, 3000)));
    }

    #[test]
    fn sincroniza_com_o_audio() {
        let t0 = Instant::now();
        let mut r = VideoReceiver::new(t0);
        for f in frags(0, true, 1000, 2000) {
            r.on_fragment(f, at(t0, 0));
        }
        // O audio a tocar vai em 900 ms: a imagem de 1000 espera.
        assert_eq!(r.pop(at(t0, 10), Some(900)), None);
        // Chega a 970 (30 ms antes, para descodificar): sai.
        let f = r.pop(at(t0, 80), Some(970)).unwrap();
        assert_eq!((f.ts_ms, r.stats.av_lag_ms), (1000, -30));
        // Audio parado muito atras: a imagem sai na mesma ao fim de 400 ms.
        for f in frags(1, false, 2000, 500) {
            r.on_fragment(f, at(t0, 100));
        }
        assert_eq!(r.pop(at(t0, 300), Some(1000)), None);
        assert!(r.pop(at(t0, 501), Some(1000)).is_some());
        // Video atrasado em relacao ao audio: sai logo.
        for f in frags(2, false, 2033, 500) {
            r.on_fragment(f, at(t0, 600));
        }
        assert!(r.pop(at(t0, 600), Some(2500)).is_some());
        assert_eq!(r.stats.av_lag_ms, 467);
    }

    #[test]
    fn relatorio_conta_perdas_debito_e_atraso_de_fila() {
        let t0 = Instant::now();
        let mut r = VideoReceiver::new(t0);
        // 10 pacotes esperados (contadores 0..9), chegam 8; 1200 bytes cada.
        for c in [0u64, 1, 2, 4, 5, 6, 8, 9] {
            r.on_packet(c, 1200);
        }
        // Imagens com atraso crescente: 20, 20, 100 ms.
        for (i, (ts, arr)) in [(0u32, 20u64), (33, 53), (66, 166)].into_iter().enumerate() {
            for f in frags(i as u32, false, ts, 100) {
                r.on_fragment(f, at(t0, arr));
            }
        }
        assert!(r.feedback(at(t0, 100), 0, Net::Mobile).is_none(), "so de 250 em 250 ms");
        let fb = r.feedback(at(t0, 250), 4, Net::Mobile).unwrap();
        assert_eq!((fb.received, fb.expected, fb.audio_loss_pct, fb.net), (8, 8, 4, Net::Mobile));
        assert_eq!(fb.recv_bps, 8 * 1200 * 8 * 4);
        // Media (20+20+100)/3 = 46, base 20: 26 ms de fila.
        assert_eq!(fb.queue_delay_ms, 26);
        for c in [10u64, 11, 14] {
            r.on_packet(c, 1200);
        }
        let fb = r.feedback(at(t0, 500), 0, Net::Mobile).unwrap();
        assert_eq!((fb.received, fb.expected), (3, 5), "do 9 ao 14: 5 esperados");
    }

    fn fb(loss_pct: u32, audio: u8, delay: u16, bps: u32) -> Feedback {
        Feedback { interval_ms: 250, received: 100 - loss_pct, expected: 100, queue_delay_ms: delay, audio_loss_pct: audio, net: Net::Unknown, recv_bps: bps, highest: None }
    }

    /// Corre o controlador `secs` segundos com relatorios a cada 250 ms.
    fn run(c: &mut RateController, t0: Instant, from_ms: u64, secs: u64, mut f: impl FnMut(u32) -> Feedback) -> u64 {
        let mut ms = from_ms;
        while ms < from_ms + secs * 1000 {
            ms += 50;
            if ms.is_multiple_of(250) {
                let r = f(c.bitrate());
                c.on_feedback(&r, false, at(t0, ms));
            }
            c.tick(at(t0, ms));
        }
        ms
    }

    #[test]
    fn pelo_relay_sobe_ate_360p_e_nao_passa() {
        let t0 = Instant::now();
        let mut c = RateController::new();
        c.set_sending(true, t0);
        let p = c.params();
        assert!(p.send && p.limited_by_relay && p.height == 360 && p.bitrate_bps == 500_000, "{p:?}");
        run(&mut c, t0, 0, 20, |b| fb(0, 0, 10, b));
        let p = c.params();
        assert_eq!((p.height, p.bitrate_bps), (360, RELAY_MAX_BPS), "{p:?}");
    }

    #[test]
    fn direto_sobe_a_1080p_com_wifi_e_fica_em_720p_com_dados() {
        let t0 = Instant::now();
        let mut c = RateController::new();
        c.set_path(true);
        c.set_local_net(Net::Wifi);
        c.set_sending(true, t0);
        assert_eq!(c.params().bitrate_bps, 1_200_000);
        let mut wifi = |b| Feedback { net: Net::Wifi, ..fb(0, 0, 10, b) };
        let ms = run(&mut c, t0, 0, 40, &mut wifi);
        let p = c.params();
        assert_eq!((p.width, p.height, p.bitrate_bps, p.limited_by_relay), (1920, 1080, 4_000_000, false), "{p:?}");
        // O outro passa para dados moveis: 720p.
        run(&mut c, t0, ms, 3, |b| Feedback { net: Net::Mobile, ..fb(0, 0, 10, b) });
        let p = c.params();
        assert_eq!((p.height, p.bitrate_bps), (720, 2_500_000), "{p:?}");
    }

    #[test]
    fn perdas_no_audio_baixam_o_video_primeiro() {
        let t0 = Instant::now();
        let mut c = RateController::new();
        c.set_path(true);
        c.set_local_net(Net::Wifi);
        c.set_sending(true, t0);
        let ms = run(&mut c, t0, 0, 30, |b| Feedback { net: Net::Wifi, ..fb(0, 0, 10, b) });
        let before = c.bitrate();
        // Video sem perdas, mas 4% de perda no audio: o video desce.
        let ms = run(&mut c, t0, ms, 3, |b| Feedback { net: Net::Wifi, ..fb(0, 4, 10, b) });
        assert!(c.bitrate() < before / 2, "{} -> {}", before, c.bitrate());
        // 1% no audio: nao desce mas tambem nao sobe.
        let held = c.bitrate();
        run(&mut c, t0, ms, 3, |b| Feedback { net: Net::Wifi, ..fb(0, 1, 10, b) });
        assert_eq!(c.bitrate(), held);
    }

    #[test]
    fn caminho_para_o_relay_desce_logo_ao_teto() {
        let t0 = Instant::now();
        let mut c = RateController::new();
        c.set_path(true);
        c.set_local_net(Net::Wifi);
        c.set_sending(true, t0);
        let ms = run(&mut c, t0, 0, 30, |b| Feedback { net: Net::Wifi, ..fb(0, 0, 10, b) });
        assert_eq!(c.params().height, 1080);
        c.set_path(false);
        let p = c.params();
        assert_eq!((p.height, p.bitrate_bps, p.limited_by_relay), (360, RELAY_MAX_BPS, true), "{p:?}");
        // Travao do mes: 240p.
        c.set_relay_mode(RelayVideo::Cap240);
        let p = c.params();
        assert_eq!((p.height, p.bitrate_bps), (240, RELAY_CAP240_BPS), "{p:?}");
        // Teto da conta: pausa pelo relay; volta a ligacao direta e continua.
        c.set_relay_mode(RelayVideo::Denied);
        assert_eq!((c.params().send, c.params().paused), (false, Some(Pause::RelayLimit)));
        // Muito tempo assim nao pausa por "rede fraca" nem estraga o debito.
        let ms = run(&mut c, t0, ms, 10, |_| Feedback { received: 0, expected: 0, ..fb(0, 0, 0, 0) });
        assert_eq!(c.params().paused, Some(Pause::RelayLimit));
        let seq = c.params().keyframe_seq;
        c.set_path(true);
        run(&mut c, t0, ms, 1, |b| Feedback { net: Net::Wifi, ..fb(0, 0, 10, b) });
        let p = c.params();
        assert!(p.send && p.paused.is_none() && p.bitrate_bps >= 1_200_000, "{p:?}");
        assert_ne!(p.keyframe_seq, seq, "recomeca com imagem completa");
    }

    #[test]
    fn rede_fraca_pausa_e_volta() {
        let t0 = Instant::now();
        let mut c = RateController::new();
        c.set_sending(true, t0);
        let seq = c.params().keyframe_seq;
        // 30% de perdas e o pouco que chega e cada vez menos: desce ate pausar.
        let ms = run(&mut c, t0, 0, 12, |b| fb(30, 0, 200, b / 2));
        let p = c.params();
        assert_eq!((p.send, p.paused), (false, Some(Pause::WeakNetwork)), "{p:?}");
        // Rede limpa (em pausa nao chega video: so conta o audio): volta em 240p, com
        // imagem completa, e fica.
        let mut resumed = None;
        let mut ms = ms;
        while ms < 30_000 {
            let sending = c.params().send;
            ms = run(&mut c, t0, ms, 1, |b| if sending { fb(0, 0, 10, b) } else { Feedback { received: 0, expected: 0, ..fb(0, 0, 0, 0) } });
            if c.params().send && resumed.is_none() {
                resumed = Some(ms);
            }
        }
        assert!(resumed.is_some_and(|r| r <= 18_000), "voltou aos {resumed:?} ms");
        let p = c.params();
        assert!(p.send && p.paused.is_none(), "{p:?}");
        assert_ne!(p.keyframe_seq, seq);
    }

    #[test]
    fn relatorio_vazio_so_e_perda_se_estiver_parado() {
        let t0 = Instant::now();
        let mut c = RateController::new();
        c.set_sending(true, t0);
        let empty = Feedback { received: 0, expected: 0, ..fb(0, 0, 0, 0) };
        // Logo a seguir a comecar: os pacotes ainda vao a caminho. Nao desce.
        for ms in [250u64, 500, 750] {
            c.on_feedback(&empty, false, at(t0, ms));
            c.tick(at(t0, ms));
        }
        assert_eq!(c.bitrate(), 500_000);
        // Pacotes enviados ha mais de 800 ms que nunca chegaram: perda.
        c.on_feedback(&empty, true, at(t0, 1_000));
        assert!(c.bitrate() < 500_000);
    }

    #[test]
    fn rede_que_recupera_nao_pausa() {
        let t0 = Instant::now();
        let mut c = RateController::new();
        c.set_sending(true, t0);
        // Desce ate ao minimo com perdas, e a rede recupera logo: nao pausa.
        let ms = run(&mut c, t0, 0, 6, |b| fb(30, 0, 10, b));
        assert_eq!(c.bitrate(), 150_000);
        run(&mut c, t0, ms, 6, |b| fb(0, 0, 10, b));
        let p = c.params();
        assert!(p.send && p.paused.is_none() && p.bitrate_bps > 150_000, "{p:?}");
    }

    #[test]
    fn sem_relatorios_pausa() {
        let t0 = Instant::now();
        let mut c = RateController::new();
        c.set_sending(true, t0);
        for ms in (0..3_900).step_by(100) {
            c.tick(at(t0, ms));
        }
        assert!(c.params().send);
        c.tick(at(t0, 4_000));
        assert_eq!(c.params().paused, Some(Pause::WeakNetwork), "relay antigo: o video nunca chega");
    }

    #[test]
    fn degrau_so_sobe_depois_de_2_s() {
        let t0 = Instant::now();
        let mut c = RateController::new();
        c.set_path(true);
        c.set_local_net(Net::Wifi);
        c.set_sending(true, t0);
        assert_eq!(c.params().height, 720, "1,2 Mbit/s no inicio: 720p");
        let mut ms = 0;
        let mut first_1080 = None;
        while ms < 30_000 && first_1080.is_none() {
            ms = run(&mut c, t0, ms, 1, |b| Feedback { net: Net::Wifi, ..fb(0, 0, 10, b) });
            if c.params().height == 1080 {
                first_1080 = Some(c.bitrate());
            }
        }
        // 1080p pede 2,2 Mbit/s x 1,15 durante 2 s (a ~8%/s, mais ~17%).
        let bps = first_1080.expect("devia chegar a 1080p");
        assert!(bps as f64 >= 2_200_000.0 * 1.15 * 1.1, "subiu a 1080p cedo demais ({bps})");
    }
}
