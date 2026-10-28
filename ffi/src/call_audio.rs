//! O audio de uma chamada, para a app (feature `calls`): o motor `qt_call` (Opus com
//! DRED, jitter buffer, relay das chamadas) por tras de um objeto UniFFI.
//!
//! A app (Kotlin) so:
//!   * chama `QtClient::call_audio_start` logo ao ligar ou ao tocar (entra ja na sala);
//!   * entrega 20 ms do microfone (`push_capture`: 960 amostras PCM 16 bits, 48 kHz,
//!     mono, em bytes little-endian, como o AudioRecord as da);
//!   * pede 20 ms para tocar (`pull_playback`, no mesmo formato).
//!
//! As chaves do audio nunca atravessam a FFI: quando a chamada e atendida (`call_accept`
//! ou a resposta do outro lado), o nucleo entrega-as ao motor. Quando a chamada acaba,
//! o motor para sozinho.
//!
//! VIDEO (fase 5b): o mesmo objeto. A app so trata da camara e do MediaCodec:
//!   * `video_params` (a cada ~250 ms): enviar ou nao, debito, resolucao, imagens/s e
//!     `keyframe_seq` (mudou: pedir ja uma imagem completa ao codificador);
//!   * `video_push`: cada imagem H.264 que sai do codificador (SPS/PPS antes das completas);
//!   * `video_pull` (thread propria): a proxima imagem do outro lado para o descodificador,
//!     ja na hora certa (sincronizada com o audio);
//!   * `video_request_keyframe`: o descodificador falhou;
//!   * `set_network`: Wi-Fi ou dados moveis (teto da ligacao direta: 1080p ou 720p).
//!
//! O que pode sair e entrar decide o nucleo (`call_video_set_camera` / `call_video_respond`):
//! sem o outro aceitar, `video_push` devolve `false` e nada sai.
//!
//! PAUSA LOCAL (0010, `video_hold`): a app saiu do ecra (segundo plano ou ecra bloqueado) com a
//! camara ligada. A camara fecha e o envio para SO deste lado, sem avisar o outro: para ele o
//! video simplesmente para (ve a minha foto ou a inicial). So pode tirar, nunca dar: com a pausa
//! nada sai, mesmo que o nucleo autorize; sem ela fica o que o nucleo autorizou.
//!
//! DIAGNOSTICO (testers, fase 5c): `diagnostic()` devolve um texto curto com os numeros da
//! chamada (caminho direto/relay, perdas, debito, resolucao, pausas, mudancas de rede), sem IPs,
//! IDs nem conteudo. Fica so no telemovel: a app mostra-o e o tester copia-o se quiser. Depois de
//! `stop()` continua a dar o resumo final da chamada.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use qt_call::codec::FRAME_SAMPLES;
use qt_call::engine::{CallEngine, EngineConfig, Net, Pause, RelayTarget, Transport};

use crate::client::{bad, QtClient, QtError};

/// Bytes de 20 ms de audio (960 amostras de 16 bits).
pub const CALL_FRAME_BYTES: u32 = (FRAME_SAMPLES * 2) as u32;

#[derive(uniffi::Object)]
pub struct CallAudio {
    engine: Mutex<Option<Arc<CallEngine>>>,
    /// O que o nucleo autorizou e a pausa local da app (ver o inicio do ficheiro).
    gate: Mutex<VideoGate>,
    /// Numeros acumulados para o diagnostico (ver o inicio do ficheiro).
    diag: Mutex<Diag>,
}

/// Contas do diagnostico: tempo em cada caminho e resolucao, pausas e mudancas de rede.
/// Amostrado sempre que a app pede `stats` ou `video_params` (varias vezes por segundo).
struct Diag {
    started: Instant,
    last: Option<Instant>,
    ms_direct: u64,
    ms_relay: u64,
    ms_both: u64,
    /// Tempo a enviar video em cada resolucao (altura) e debito pedido (bps x ms).
    ms_by_height: BTreeMap<u16, u64>,
    ms_sending: u64,
    bitrate_ms: u128,
    bitrate_min: u32,
    bitrate_max: u32,
    ms_limited_by_relay: u64,
    pause: Option<Pause>,
    pauses_weak: u32,
    pauses_relay: u32,
    net: Option<Net>,
    net_changes: u32,
    transport: Transport,
    transport_changes: u32,
    /// Resumo final (guardado no `stop`).
    final_text: Option<String>,
}

impl Diag {
    fn new() -> Self {
        Diag {
            started: Instant::now(),
            last: None,
            ms_direct: 0,
            ms_relay: 0,
            ms_both: 0,
            ms_by_height: BTreeMap::new(),
            ms_sending: 0,
            bitrate_ms: 0,
            bitrate_min: u32::MAX,
            bitrate_max: 0,
            ms_limited_by_relay: 0,
            pause: None,
            pauses_weak: 0,
            pauses_relay: 0,
            net: None,
            net_changes: 0,
            transport: Transport::Connecting,
            transport_changes: 0,
            final_text: None,
        }
    }

    fn sample(&mut self, e: &CallEngine) {
        let now = Instant::now();
        let dt = self.last.map(|l| now.duration_since(l).as_millis() as u64).unwrap_or(0).min(2_000);
        self.last = Some(now);
        let s = e.stats();
        if s.transport != self.transport {
            if self.transport != Transport::Connecting {
                self.transport_changes += 1;
            }
            self.transport = s.transport;
        }
        if s.joined && s.peer_present && e.has_media() {
            if s.both_paths {
                self.ms_both += dt;
            } else if s.direct {
                self.ms_direct += dt;
            } else {
                self.ms_relay += dt;
            }
        }
        let p = e.video_params();
        if p.send {
            self.ms_sending += dt;
            *self.ms_by_height.entry(p.height).or_default() += dt;
            self.bitrate_ms += p.bitrate_bps as u128 * dt as u128;
            self.bitrate_min = self.bitrate_min.min(p.bitrate_bps);
            self.bitrate_max = self.bitrate_max.max(p.bitrate_bps);
            if p.limited_by_relay {
                self.ms_limited_by_relay += dt;
            }
        }
        if p.paused != self.pause {
            match p.paused {
                Some(Pause::WeakNetwork) => self.pauses_weak += 1,
                Some(Pause::RelayLimit) => self.pauses_relay += 1,
                None => {}
            }
            self.pause = p.paused;
        }
    }

    fn set_net(&mut self, n: Net) {
        if self.net.is_some_and(|old| old != n) {
            self.net_changes += 1;
        }
        self.net = Some(n);
    }

    fn text(&self, e: &CallEngine) -> String {
        fn pct(part: u64, total: u64) -> String {
            match (part * 100 + total / 2).checked_div(total) {
                Some(v) => format!("{v}%"),
                None => "0%".into(),
            }
        }
        fn permille(part: u64, total: u64) -> String {
            if total == 0 { "0%".into() } else { format!("{:.1}%", part as f64 * 100.0 / total as f64) }
        }
        let s = e.stats();
        let v = e.video_stats();
        let secs = self.started.elapsed().as_secs();
        let mut t = String::new();
        t += &format!("duracao: {}m{:02}s\n", secs / 60, secs % 60);
        let path_total = self.ms_direct + self.ms_relay + self.ms_both;
        t += &format!(
            "caminho: direto {} · relay {} · os dois {} · {}\n",
            pct(self.ms_direct, path_total),
            pct(self.ms_relay, path_total),
            pct(self.ms_both, path_total),
            match s.transport {
                Transport::Udp => "udp",
                Transport::WebSocket => "websocket",
                Transport::Connecting => "a ligar",
            },
        );
        if self.transport_changes > 0 {
            t += &format!("trocas udp/websocket: {}\n", self.transport_changes);
        }
        if let Some(rtt) = s.relay_rtt_ms {
            t += &format!("ida e volta ao relay: {rtt} ms\n");
        }
        t += &format!(
            "rede: {} · mudancas {}\n",
            match self.net {
                Some(Net::Wifi) => "wi-fi",
                Some(Net::Mobile) => "dados moveis",
                _ => "?",
            },
            self.net_changes,
        );
        let lost = s.jitter.lost;
        t += &format!(
            "voz: recebidos {} · perdidos {} ({}) · tardios {} · dred {} · plc {} · jitter {} ms · buffer {} ms\n",
            s.received,
            lost,
            permille(lost, s.received + lost),
            s.jitter.gave_up,
            s.dred_frames,
            s.plc_frames,
            s.jitter.jitter_ms,
            s.jitter.depth_ms,
        );
        if self.ms_sending > 0 || v.sent_frames > 0 {
            let res: Vec<String> = self
                .ms_by_height
                .iter()
                .rev()
                .map(|(h, ms)| format!("{h}p {}", pct(*ms, self.ms_sending)))
                .collect();
            let avg = if self.ms_sending > 0 { (self.bitrate_ms / self.ms_sending as u128) as u32 } else { 0 };
            t += &format!(
                "video enviado: {} · {} kb/s (min {} max {}) · limitado pelo relay {} · imagens {} · deitadas fora {}\n",
                if res.is_empty() { "-".into() } else { res.join(" ") },
                avg / 1000,
                if self.bitrate_min == u32::MAX { 0 } else { self.bitrate_min / 1000 },
                self.bitrate_max / 1000,
                pct(self.ms_limited_by_relay, self.ms_sending),
                v.sent_frames,
                v.dropped_frames,
            );
            t += &format!(
                "video enviado, do outro lado: perda {:.1}% · relatorios {} · sem chegar nada {}\n",
                v.peer_loss_permille as f64 / 10.0,
                v.feedbacks,
                v.stalls,
            );
        }
        if v.recv.frames > 0 || v.recv.lost_frames > 0 {
            let r = &v.recv;
            t += &format!(
                "video recebido: imagens {} · perdidas {} ({}) · completas {} · pedidos {} · fila {} ms · atras do audio {} ms\n",
                r.frames,
                r.lost_frames,
                permille(r.lost_frames, r.frames + r.lost_frames),
                r.keyframes,
                r.keyframe_requests,
                r.queue_delay_ms,
                r.av_lag_ms,
            );
        }
        if self.pauses_weak + self.pauses_relay > 0 {
            t += &format!("pausas do video: rede fraca {} · limite do relay {}\n", self.pauses_weak, self.pauses_relay);
        }
        if v.fichas_spent > 0 || v.relay_mode != 0 {
            t += &format!("fichas gastas: {} · relay: {}\n", v.fichas_spent, ["normal", "so 240p", "sem video"].get(v.relay_mode as usize).unwrap_or(&"?"));
        }
        t
    }
}

/// Enviar video: o que o nucleo autorizou e a pausa local (`video_hold`).
#[derive(Default)]
struct VideoGate {
    send: bool,
    hold: bool,
}

/// Rede do telemovel, para o teto do video em ligacao direta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum NetworkKind {
    Unknown,
    Wifi,
    Mobile,
}

/// Porque o video esta em pausa (a voz continua).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VideoPause {
    /// "Video em pausa: rede fraca."
    WeakNetwork,
    /// Pelo servidor ja nao ha video este mes (teto da conta); em ligacao direta continua.
    RelayLimit,
}

/// O que o codificador deve fazer agora.
#[derive(Debug, Clone, uniffi::Record)]
pub struct VideoEncoderParams {
    /// Enviar imagens agora (o outro aceitou e nao ha pausa).
    pub send: bool,
    pub bitrate_bps: u32,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub paused: Option<VideoPause>,
    /// A qualidade esta limitada por ir pelo servidor (ligacao direta daria mais).
    pub limited_by_relay: bool,
    /// Mudou desde a ultima vez: pedir ja uma imagem completa ao codificador.
    pub keyframe_seq: u32,
}

/// Uma imagem do outro lado, pronta para o descodificador (H.264, Annex B).
#[derive(Debug, Clone, uniffi::Record)]
pub struct VideoFrameIn {
    pub data: Vec<u8>,
    pub keyframe: bool,
    /// Rodar tanto (0, 90, 180 ou 270 graus, sentido dos ponteiros) para a mostrar direita.
    pub rotation_degrees: u32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CallAudioStats {
    /// "udp", "websocket" ou "a ligar".
    pub transport: String,
    pub joined: bool,
    /// O outro telemovel ja esta na sala.
    pub peer_present: bool,
    /// Ida e volta ate ao relay das chamadas, em ms.
    pub relay_rtt_ms: Option<u32>,
    /// O audio que envio vai direto ao outro telemovel (ligacao direta, so entre contactos
    /// mutuos). `false`: pelo relay. So para diagnostico: a app nao mostra nada disto.
    pub direct: bool,
    /// ... e tambem pelo servidor: o caminho direto falhou respostas ha pouco (ainda a decidir).
    pub both_paths: bool,
    pub received: u64,
    /// Frames perdidos na rede (reconstruidos pelo DRED ou preenchidos pelo PLC).
    pub lost: u64,
    /// Frames que nao chegaram a tempo de tocar (o microfone ou a rede do outro lado
    /// pararam mais de 200 ms): preenchidos pelo PLC. Nao sao perdas da rede.
    pub late: u64,
    pub dred_frames: u64,
    pub plc_frames: u64,
    /// Variacao das chegadas e atraso atual do jitter buffer, em ms.
    pub jitter_ms: u32,
    pub buffer_ms: u32,
    /// Video: imagens recebidas e perdidas, e o que saiu deste telemovel (kB).
    pub video_frames_in: u64,
    pub video_lost_frames: u64,
    pub video_sent_kb: u64,
}

impl CallAudio {
    fn sample(&self) {
        if let (Some(e), Ok(mut d)) = (self.engine(), self.diag.lock()) {
            d.sample(&e);
        }
    }

    pub(crate) fn new(engine: CallEngine) -> Arc<Self> {
        Arc::new(CallAudio {
            engine: Mutex::new(Some(Arc::new(engine))),
            gate: Mutex::new(VideoGate::default()),
            diag: Mutex::new(Diag::new()),
        })
    }

    fn engine(&self) -> Option<Arc<CallEngine>> {
        self.engine.lock().ok().and_then(|e| e.clone())
    }

    /// As chaves do audio (chamada atendida).
    pub(crate) fn attach(&self, media: qt_core::call::MediaSession) {
        if let Some(e) = self.engine() {
            e.attach_media(media);
        }
    }

    /// Ligacao direta: so `crate::call` chama isto, com a regra cumprida (ver la).
    /// O nucleo: o que pode sair (camara ligada E o outro aceitou) e entrar (aceitei E a
    /// camara dele esta ligada).
    /// Com a pausa local (`video_hold`) nada sai, mesmo autorizado.
    pub(crate) fn video_authorize(&self, send: bool, receive: bool) {
        // O motor muda com o `gate` preso: uma pausa e uma autorizacao nunca se cruzam.
        let Ok(mut g) = self.gate.lock() else { return };
        g.send = send;
        if let Some(e) = self.engine() {
            e.video_set_sending(send && !g.hold);
            e.video_set_receiving(receive);
        }
    }

    pub(crate) fn p2p_enable(&self) {
        if let Some(e) = self.engine() {
            e.p2p_enable();
        }
    }

    pub(crate) fn p2p_disable(&self) {
        if let Some(e) = self.engine() {
            e.p2p_disable();
        }
    }

    pub(crate) fn p2p_local_candidates(&self) -> Option<Vec<std::net::SocketAddr>> {
        self.engine().and_then(|e| e.p2p_local_candidates())
    }

    pub(crate) fn p2p_set_peer(&self, peer: Vec<std::net::SocketAddr>) {
        if let Some(e) = self.engine() {
            e.p2p_set_peer(peer);
        }
    }
}

#[uniffi::export]
impl CallAudio {
    /// 20 ms do microfone (1920 bytes). Antes de atender ou depois de parar: ignorado.
    pub fn push_capture(&self, pcm: Vec<u8>) -> Result<(), QtError> {
        if pcm.len() != CALL_FRAME_BYTES as usize {
            return Err(bad("pcm"));
        }
        let Some(e) = self.engine() else { return Ok(()) };
        let samples: Vec<i16> = pcm.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        e.push_capture(&samples).map_err(|_| QtError::Crypto)
    }

    /// Os proximos 20 ms para o altifalante (1920 bytes; silencio se a chamada parou).
    pub fn pull_playback(&self) -> Vec<u8> {
        let mut out = vec![0i16; FRAME_SAMPLES];
        if let Some(e) = self.engine() {
            let _ = e.pull_playback(&mut out);
        }
        out.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    /// O que o codificador deve fazer agora (perguntar a cada ~250 ms).
    pub fn video_params(&self) -> VideoEncoderParams {
        self.sample();
        let p = self.engine().map(|e| e.video_params());
        match p {
            Some(p) => VideoEncoderParams {
                send: p.send,
                bitrate_bps: p.bitrate_bps,
                width: p.width as u32,
                height: p.height as u32,
                fps: p.fps as u32,
                paused: p.paused.map(|r| match r {
                    Pause::WeakNetwork => VideoPause::WeakNetwork,
                    Pause::RelayLimit => VideoPause::RelayLimit,
                }),
                limited_by_relay: p.limited_by_relay,
                keyframe_seq: p.keyframe_seq,
            },
            None => VideoEncoderParams {
                send: false,
                bitrate_bps: 0,
                width: 0,
                height: 0,
                fps: 0,
                paused: None,
                limited_by_relay: true,
                keyframe_seq: 0,
            },
        }
    }

    /// Uma imagem H.264 do codificador. `rotation_degrees`: quanto rodar (sentido dos
    /// ponteiros) para a ver direita; `age_ms`: ha quanto tempo foi captada. `false`: nao
    /// saiu (o outro ainda nao aceitou, pausa, ou rede sem espaco: pedir imagem completa).
    pub fn video_push(&self, frame: Vec<u8>, keyframe: bool, rotation_degrees: u32, age_ms: u32) -> bool {
        let quarter = ((rotation_degrees % 360) / 90) as u8;
        self.engine().is_some_and(|e| e.video_push(&frame, keyframe, quarter, age_ms))
    }

    /// A proxima imagem do outro lado, quando for a hora dela. Espera ate `timeout_ms`.
    pub fn video_pull(&self, timeout_ms: u32) -> Option<VideoFrameIn> {
        let e = self.engine()?;
        e.video_pull(std::time::Duration::from_millis(timeout_ms.min(1_000) as u64))
            .map(|f| VideoFrameIn { data: f.data, keyframe: f.keyframe, rotation_degrees: f.rotation as u32 * 90 })
    }

    /// O descodificador falhou: pede ao outro lado uma imagem completa.
    pub fn video_request_keyframe(&self) {
        if let Some(e) = self.engine() {
            e.video_request_keyframe();
        }
    }

    /// PAUSA LOCAL da minha camara (a app saiu do ecra: segundo plano ou ecra bloqueado). `true`:
    /// nada sai, sem avisar o outro lado (para ele o video para, como numa falha de rede).
    /// `false`: volta ao que o nucleo autorizou (com imagem completa logo a seguir). Nunca
    /// autoriza nada sozinha: o consentimento continua a ser do nucleo.
    pub fn video_hold(&self, on: bool) {
        let Ok(mut g) = self.gate.lock() else { return };
        if g.hold == on {
            return;
        }
        g.hold = on;
        if let Some(e) = self.engine() {
            e.video_set_sending(g.send && !on);
        }
    }

    /// Rede do telemovel (Wi-Fi ou dados moveis).
    pub fn set_network(&self, kind: NetworkKind) {
        if let Ok(mut d) = self.diag.lock() {
            d.set_net(match kind {
                NetworkKind::Unknown => Net::Unknown,
                NetworkKind::Wifi => Net::Wifi,
                NetworkKind::Mobile => Net::Mobile,
            });
        }
        if let Some(e) = self.engine() {
            e.set_network(match kind {
                NetworkKind::Unknown => Net::Unknown,
                NetworkKind::Wifi => Net::Wifi,
                NetworkKind::Mobile => Net::Mobile,
            });
        }
    }

    pub fn stats(&self) -> CallAudioStats {
        self.sample();
        let s = self.engine().map(|e| e.stats()).unwrap_or_default();
        let v = self.engine().map(|e| e.video_stats()).unwrap_or_default();
        CallAudioStats {
            transport: match s.transport {
                Transport::Udp => "udp",
                Transport::WebSocket => "websocket",
                Transport::Connecting => "a ligar",
            }
            .to_string(),
            joined: s.joined,
            peer_present: s.peer_present,
            relay_rtt_ms: s.relay_rtt_ms,
            direct: s.direct,
            both_paths: s.both_paths,
            received: s.received,
            lost: s.jitter.lost,
            late: s.jitter.gave_up,
            dred_frames: s.dred_frames,
            plc_frames: s.plc_frames,
            jitter_ms: s.jitter.jitter_ms,
            buffer_ms: s.jitter.depth_ms,
            video_frames_in: v.recv.frames,
            video_lost_frames: v.recv.lost_frames,
            video_sent_kb: v.sent_bytes / 1000,
        }
    }

    /// Ja tem as chaves do audio (a chamada foi atendida)?
    pub fn active(&self) -> bool {
        self.engine().is_some_and(|e| e.has_media())
    }

    /// Para o motor (fecha a ligacao ao relay das chamadas). Pode ser chamado varias vezes.
    pub fn stop(&self) {
        let e = self.engine.lock().ok().and_then(|mut e| e.take());
        if let (Some(e), Ok(mut d)) = (e.as_ref(), self.diag.lock()) {
            if d.final_text.is_none() {
                d.sample(e);
                d.final_text = Some(d.text(e));
            }
        }
        drop(e);
    }

    /// DIAGNOSTICO para os testers (ver o inicio do ficheiro): os numeros desta chamada, num
    /// texto curto, sem IPs, IDs nem conteudo. Depois de `stop()`, o resumo final.
    pub fn diagnostic(&self) -> String {
        match self.engine() {
            Some(e) => match self.diag.lock() {
                Ok(mut d) => {
                    d.sample(&e);
                    d.text(&e)
                }
                Err(_) => String::new(),
            },
            None => self.diag.lock().ok().and_then(|d| d.final_text.clone()).unwrap_or_default(),
        }
    }
}

#[uniffi::export]
impl QtClient {
    /// AUDIO da chamada: arranca o motor (Opus, jitter buffer, relay das chamadas) e entra
    /// ja na sala, mesmo antes de atender (pre-aquecimento). Chamar ao ligar ou quando
    /// toca. `relay_url` e o do relay das mensagens ("wss://relay.sealer.pt") e
    /// `relay_public_key` a chave dele (o pin, 32 bytes). Chamar outra vez devolve o mesmo.
    pub fn call_audio_start(&self, call_id: String, relay_url: String, relay_public_key: Vec<u8>) -> Result<Arc<CallAudio>, QtError> {
        let id: [u8; 16] = crate::client::unhex(&call_id).and_then(|b| b.try_into().ok()).ok_or_else(|| bad("call_id"))?;
        if relay_public_key.len() != 32 {
            return Err(bad("relay_public_key"));
        }
        // As fichas do mes (base cifrada) para o motor, antes de fechar as chamadas (a base e
        // sempre aberta antes das chamadas, nunca o contrario). So para um motor novo.
        let started = crate::client::relock(&self.calls).has_engine(&id);
        let wallet = if started { None } else { Some(crate::fichas::wallet_for_call(self)?) };
        let mut calls = crate::client::relock(&self.calls);
        crate::call::start_for_call(&mut calls, id, &relay_url, &relay_public_key, wallet)
    }
}

/// Bytes de 20 ms de audio que `push_capture` espera e `pull_playback` devolve.
#[uniffi::export]
pub fn call_frame_bytes() -> u32 {
    CALL_FRAME_BYTES
}

/// Onde esta o relay das chamadas, a partir do URL do relay das mensagens
/// ("wss://relay.sealer.pt" -> UDP "relay.sealer.pt:443" e "wss://relay.sealer.pt/call").
pub(crate) fn relay_target(relay_url: &str) -> Result<RelayTarget, QtError> {
    let (scheme, rest) = relay_url.split_once("://").ok_or_else(|| bad("relay_url"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    if host.is_empty() || !(scheme == "wss" || scheme == "ws") {
        return Err(bad("relay_url"));
    }
    let udp_host = if host.contains(':') { format!("[{host}]") } else { host.to_string() };
    Ok(RelayTarget { udp: format!("{udp_host}:443"), ws_url: Some(format!("{scheme}://{authority}/call")) })
}

pub(crate) fn engine_config(relay_url: &str) -> Result<EngineConfig, QtError> {
    #[allow(unused_mut)]
    let mut cfg = EngineConfig::new(relay_target(relay_url)?);
    // Testes desta crate: os dois telemoveis estao no mesmo PC (127.0.0.1).
    #[cfg(test)]
    {
        cfg.p2p_allow_loopback = true;
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_das_chamadas_a_partir_do_url() {
        let t = relay_target("wss://relay.sealer.pt").unwrap();
        assert_eq!((t.udp.as_str(), t.ws_url.as_deref()), ("relay.sealer.pt:443", Some("wss://relay.sealer.pt/call")));
        let t = relay_target("wss://relay.sealer.pt:443/").unwrap();
        assert_eq!(t.ws_url.as_deref(), Some("wss://relay.sealer.pt:443/call"));
        assert!(relay_target("https://x").is_err());
        assert!(relay_target("nada").is_err());
    }
}
