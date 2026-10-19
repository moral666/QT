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

use std::sync::{Arc, Mutex};

use qt_call::codec::FRAME_SAMPLES;
use qt_call::engine::{CallEngine, EngineConfig, RelayTarget, Transport};

use crate::client::{bad, QtClient, QtError};

/// Bytes de 20 ms de audio (960 amostras de 16 bits).
pub const CALL_FRAME_BYTES: u32 = (FRAME_SAMPLES * 2) as u32;

#[derive(uniffi::Object)]
pub struct CallAudio {
    engine: Mutex<Option<Arc<CallEngine>>>,
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
    pub received: u64,
    /// Frames perdidos pela rede (reconstruidos pelo DRED ou preenchidos pelo PLC).
    pub lost: u64,
    pub dred_frames: u64,
    pub plc_frames: u64,
    /// Variacao das chegadas e atraso atual do jitter buffer, em ms.
    pub jitter_ms: u32,
    pub buffer_ms: u32,
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
}

#[uniffi::export]
impl CallAudio {
    /// 20 ms do microfone (1920 bytes). Antes de atender ou depois de parar: ignorado.
    pub fn push_capture(&self, pcm: Vec<u8>) -> Result<(), QtError> {
        if pcm.len() != CALL_FRAME_BYTES as usize {
            return Err(bad("pcm"));
        }
        let Some(e) = self.engine() else { return Ok(()) };
        let samples: Vec<i16> = pcm.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
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

    pub fn stats(&self) -> CallAudioStats {
        let s = self.engine().map(|e| e.stats()).unwrap_or_default();
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
            received: s.received,
            lost: s.jitter.lost,
            dred_frames: s.dred_frames,
            plc_frames: s.plc_frames,
            jitter_ms: s.jitter.jitter_ms,
            buffer_ms: s.jitter.depth_ms,
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
        let mut calls = self.calls.lock().map_err(|_| QtError::Storage)?;
        crate::call::start_for_call(&mut calls, id, &relay_url, &relay_public_key)
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
    Ok(EngineConfig::new(relay_target(relay_url)?))
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
