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

use std::sync::{Arc, Mutex};

use qt_call::codec::FRAME_SAMPLES;
use qt_call::engine::{CallEngine, EngineConfig, Net, Pause, RelayTarget, Transport};

use crate::client::{bad, QtClient, QtError};

/// Bytes de 20 ms de audio (960 amostras de 16 bits).
pub const CALL_FRAME_BYTES: u32 = (FRAME_SAMPLES * 2) as u32;

#[derive(uniffi::Object)]
pub struct CallAudio {
    engine: Mutex<Option<Arc<CallEngine>>>,
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
    pub(crate) fn new(engine: CallEngine) -> Arc<Self> {
        Arc::new(CallAudio { engine: Mutex::new(Some(Arc::new(engine))) })
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
    pub(crate) fn video_authorize(&self, send: bool, receive: bool) {
        if let Some(e) = self.engine() {
            e.video_set_sending(send);
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

    /// Rede do telemovel (Wi-Fi ou dados moveis).
    pub fn set_network(&self, kind: NetworkKind) {
        if let Some(e) = self.engine() {
            e.set_network(match kind {
                NetworkKind::Unknown => Net::Unknown,
                NetworkKind::Wifi => Net::Wifi,
                NetworkKind::Mobile => Net::Mobile,
            });
        }
    }

    pub fn stats(&self) -> CallAudioStats {
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
        drop(e);
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
