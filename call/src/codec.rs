//! Opus 1.6.1 (libopus oficial, em C, pelo `opusic-sys`), com DRED e PLC neural.
//!
//! Escolhas (aprovadas pelo Nuno a 19/10/2026, "A1 + B1"):
//!   * 48 kHz mono, frames de 20 ms (960 amostras), voz em banda larga (SILK 16 kHz).
//!   * VBR com restricao e um teto de `MAX_OPUS_BYTES`: o pacote cifrado tem tamanho
//!     fixo na mesma (`qt_core::call`), por isso o tamanho do Opus nao se ve na rede.
//!   * Sem DTX (o silencio tambem gera pacotes: o ritmo nao revela quando se fala).
//!   * Sem o FEC interno (LBRR): o DRED substitui-o, como recomenda o libopus.
//!   * DRED: cada pacote leva uma versao muito comprimida do audio anterior (ate
//!     `dred_duration` x 10 ms). O libopus so o gera com uma perda esperada > 0 e tira
//!     esses bits ao debito total (por isso o debito pedido e maior que os ~24 kbit/s
//!     da voz). Quem recebe usa-o para reconstruir pacotes perdidos.
//!   * PLC neural (complexidade do descodificador >= 5): quando nao ha nada para
//!     reconstruir, o buraco e preenchido por um modelo, em vez de um "zumbido".

use opusic_sys as ffi;
use std::os::raw::c_int;
use std::ptr::NonNull;

pub const SAMPLE_RATE: i32 = 48_000;
pub const FRAME_MS: u64 = 20;
/// Amostras por frame (20 ms a 48 kHz, mono).
pub const FRAME_SAMPLES: usize = 960;
/// Teto de cada frame Opus: cabe no tamanho fixo da chamada (`pad` 128 = 2 + 126).
pub const MAX_OPUS_BYTES: usize = 126;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CodecError {
    #[error("erro do libopus: {0}")]
    Opus(i32),
    #[error("o frame tem de ter 960 amostras (20 ms a 48 kHz)")]
    FrameSize,
}

fn check(ret: c_int) -> Result<c_int, CodecError> {
    if ret < 0 {
        Err(CodecError::Opus(ret))
    } else {
        Ok(ret)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EncoderConfig {
    /// Debito total pedido ao Opus (voz + DRED), em bit/s.
    pub bitrate: i32,
    /// 0 a 10. Mais alto = melhor qualidade e mais processador.
    pub complexity: i32,
    /// Perda esperada (%). Tem de ser > 0 para haver DRED.
    pub loss_percent: i32,
    /// Duracao da redundancia DRED, em unidades de 10 ms (50 = 500 ms; maximo 104).
    pub dred_duration: i32,
    /// Teto de bytes por frame.
    pub max_packet: usize,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        EncoderConfig { bitrate: 36_000, complexity: 8, loss_percent: 10, dred_duration: 50, max_packet: MAX_OPUS_BYTES }
    }
}

pub struct Encoder {
    st: NonNull<ffi::OpusEncoder>,
    max_packet: usize,
}

// O estado do libopus so e usado por quem tem o `Encoder` (&mut); pode mudar de thread.
unsafe impl Send for Encoder {}

impl Encoder {
    pub fn new(cfg: EncoderConfig) -> Result<Self, CodecError> {
        let mut err: c_int = 0;
        // SAFETY: argumentos validos (48 kHz, 1 canal, aplicacao VOIP); o ponteiro e
        // verificado e fica dono do `Encoder`, que o liberta no Drop.
        let st = unsafe { ffi::opus_encoder_create(SAMPLE_RATE, 1, ffi::OPUS_APPLICATION_VOIP, &mut err) };
        let st = NonNull::new(st).ok_or(CodecError::Opus(err))?;
        let enc = Encoder { st, max_packet: cfg.max_packet.min(MAX_OPUS_BYTES) };
        enc.ctl(ffi::OPUS_SET_BITRATE_REQUEST, cfg.bitrate)?;
        enc.ctl(ffi::OPUS_SET_MAX_BANDWIDTH_REQUEST, ffi::OPUS_BANDWIDTH_WIDEBAND)?;
        enc.ctl(ffi::OPUS_SET_SIGNAL_REQUEST, ffi::OPUS_SIGNAL_VOICE)?;
        enc.ctl(ffi::OPUS_SET_VBR_REQUEST, 1)?;
        enc.ctl(ffi::OPUS_SET_VBR_CONSTRAINT_REQUEST, 1)?;
        enc.ctl(ffi::OPUS_SET_COMPLEXITY_REQUEST, cfg.complexity)?;
        enc.ctl(ffi::OPUS_SET_INBAND_FEC_REQUEST, 0)?;
        enc.ctl(ffi::OPUS_SET_DTX_REQUEST, 0)?;
        enc.ctl(ffi::OPUS_SET_PACKET_LOSS_PERC_REQUEST, cfg.loss_percent.clamp(1, 100))?;
        enc.ctl(ffi::OPUS_SET_DRED_DURATION_REQUEST, cfg.dred_duration.clamp(0, 104))?;
        Ok(enc)
    }

    fn ctl(&self, request: c_int, value: i32) -> Result<(), CodecError> {
        // SAFETY: pedidos SET com um opus_int32, como o libopus espera.
        check(unsafe { ffi::opus_encoder_ctl(self.st.as_ptr(), request, value) })?;
        Ok(())
    }

    /// Ajusta a perda esperada (mais perda = mais bits para o DRED).
    pub fn set_loss_percent(&mut self, percent: i32) -> Result<(), CodecError> {
        self.ctl(ffi::OPUS_SET_PACKET_LOSS_PERC_REQUEST, percent.clamp(1, 100))
    }

    /// Codifica 20 ms (960 amostras). Devolve os bytes escritos em `out`.
    pub fn encode(&mut self, pcm: &[i16], out: &mut [u8]) -> Result<usize, CodecError> {
        if pcm.len() != FRAME_SAMPLES {
            return Err(CodecError::FrameSize);
        }
        let max = self.max_packet.min(out.len());
        // SAFETY: `pcm` tem 960 amostras e `out` tem pelo menos `max` bytes.
        let n = unsafe {
            ffi::opus_encode(self.st.as_ptr(), pcm.as_ptr(), FRAME_SAMPLES as c_int, out.as_mut_ptr(), max as i32)
        };
        Ok(check(n)? as usize)
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: criado por opus_encoder_create e libertado uma so vez.
        unsafe { ffi::opus_encoder_destroy(self.st.as_ptr()) }
    }
}

/// Descodificador com PLC neural e reconstrucao pelo DRED.
pub struct Decoder {
    st: NonNull<ffi::OpusDecoder>,
    dred_dec: NonNull<ffi::OpusDREDDecoder>,
    dred: NonNull<ffi::OpusDRED>,
    /// Amostras de redundancia disponiveis no ultimo `parse_dred`.
    dred_available: i32,
}

unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new() -> Result<Self, CodecError> {
        let mut err: c_int = 0;
        // SAFETY: como no Encoder; cada ponteiro e verificado antes de ser usado.
        let st = unsafe { ffi::opus_decoder_create(SAMPLE_RATE, 1, &mut err) };
        let st = NonNull::new(st).ok_or(CodecError::Opus(err))?;
        let dred_dec = unsafe { ffi::opus_dred_decoder_create(&mut err) };
        let Some(dred_dec) = NonNull::new(dred_dec) else {
            unsafe { ffi::opus_decoder_destroy(st.as_ptr()) };
            return Err(CodecError::Opus(err));
        };
        let dred = unsafe { ffi::opus_dred_alloc(&mut err) };
        let Some(dred) = NonNull::new(dred) else {
            unsafe {
                ffi::opus_dred_decoder_destroy(dred_dec.as_ptr());
                ffi::opus_decoder_destroy(st.as_ptr());
            }
            return Err(CodecError::Opus(err));
        };
        let d = Decoder { st, dred_dec, dred, dred_available: 0 };
        // Complexidade 5: liga o PLC neural (os modelos estao compilados).
        // SAFETY: pedido SET com um opus_int32.
        check(unsafe { ffi::opus_decoder_ctl(d.st.as_ptr(), ffi::OPUS_SET_COMPLEXITY_REQUEST, 5i32) })?;
        Ok(d)
    }

    /// Descodifica um frame recebido para `out` (960 amostras).
    pub fn decode(&mut self, packet: &[u8], out: &mut [i16]) -> Result<usize, CodecError> {
        if out.len() < FRAME_SAMPLES {
            return Err(CodecError::FrameSize);
        }
        // SAFETY: `out` tem espaco para 960 amostras; o pacote e lido so ate `len`.
        let n = unsafe {
            ffi::opus_decode(self.st.as_ptr(), packet.as_ptr(), packet.len() as i32, out.as_mut_ptr(), FRAME_SAMPLES as c_int, 0)
        };
        Ok(check(n)? as usize)
    }

    /// Nao ha nada para este frame: o PLC (neural) inventa a continuacao.
    pub fn conceal(&mut self, out: &mut [i16]) -> Result<usize, CodecError> {
        if out.len() < FRAME_SAMPLES {
            return Err(CodecError::FrameSize);
        }
        // SAFETY: pacote nulo com tamanho 0 = pedido de PLC.
        let n = unsafe {
            ffi::opus_decode(self.st.as_ptr(), std::ptr::null(), 0, out.as_mut_ptr(), FRAME_SAMPLES as c_int, 0)
        };
        Ok(check(n)? as usize)
    }

    /// Le a redundancia (DRED) que vem em `packet`, para reconstruir ate `frames_back`
    /// frames que se perderam ANTES dele. Devolve quantos frames ficam cobertos.
    pub fn parse_dred(&mut self, packet: &[u8], frames_back: u32) -> u32 {
        // Pede-se um pouco mais: o DRED vem em blocos de 40 ms e o inicio pode estar
        // desalinhado com os frames.
        let wanted = ((frames_back as i32 + 2) * FRAME_SAMPLES as i32).min(SAMPLE_RATE);
        let mut end: c_int = 0;
        // SAFETY: estados criados no `new`; o pacote e lido so ate `len`.
        let r = unsafe {
            ffi::opus_dred_parse(
                self.dred_dec.as_ptr(),
                self.dred.as_ptr(),
                packet.as_ptr(),
                packet.len() as i32,
                wanted,
                SAMPLE_RATE,
                &mut end,
                0,
            )
        };
        self.dred_available = r.max(0);
        (self.dred_available / FRAME_SAMPLES as i32) as u32
    }

    /// Reconstroi o frame que comecava `frames_back` frames antes do pacote lido no
    /// ultimo `parse_dred`. Sem redundancia que chegue, faz PLC. Devolve `true` se
    /// veio mesmo do DRED.
    pub fn decode_dred(&mut self, frames_back: u32, out: &mut [i16]) -> Result<bool, CodecError> {
        let offset = frames_back as i32 * FRAME_SAMPLES as i32;
        if frames_back == 0 || offset > self.dred_available || out.len() < FRAME_SAMPLES {
            self.conceal(out)?;
            return Ok(false);
        }
        // SAFETY: `dred` foi preenchido pelo `parse_dred`; `out` tem 960 amostras.
        let n = unsafe {
            ffi::opus_decoder_dred_decode(self.st.as_ptr(), self.dred.as_ptr(), offset, out.as_mut_ptr(), FRAME_SAMPLES as c_int)
        };
        check(n)?;
        Ok(true)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: cada estado e libertado uma so vez, pela funcao certa.
        unsafe {
            ffi::opus_dred_free(self.dred.as_ptr());
            ffi::opus_dred_decoder_destroy(self.dred_dec.as_ptr());
            ffi::opus_decoder_destroy(self.st.as_ptr());
        }
    }
}

/// Versao do libopus ligado (ex.: "libopus 1.6.1").
pub fn opus_version() -> String {
    // SAFETY: devolve uma string estatica terminada em zero.
    unsafe { std::ffi::CStr::from_ptr(ffi::opus_get_version_string()) }.to_string_lossy().into_owned()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// "Voz" sintetica: harmonicos de uma fundamental que varia, com silabas (envolvente).
    pub(crate) fn speech_like(frame: usize) -> Vec<i16> {
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

    pub(crate) fn energy(x: &[i16]) -> f64 {
        x.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / x.len() as f64
    }

    #[test]
    fn versao_e_1_6_1() {
        assert!(opus_version().contains("1.6.1"), "{}", opus_version());
    }

    #[test]
    fn codifica_e_descodifica_dentro_do_teto() {
        let mut enc = Encoder::new(EncoderConfig::default()).unwrap();
        let mut dec = Decoder::new().unwrap();
        let mut buf = [0u8; 400];
        let mut out = [0i16; FRAME_SAMPLES];
        let (mut total, mut max) = (0usize, 0usize);
        for f in 0..100 {
            let n = enc.encode(&speech_like(f), &mut buf).unwrap();
            assert!(n <= MAX_OPUS_BYTES);
            total += n;
            max = max.max(n);
            assert_eq!(dec.decode(&buf[..n], &mut out).unwrap(), FRAME_SAMPLES);
        }
        assert!(energy(&out) > 1e4, "saida sem som");
        // 36 kbit/s em 20 ms ~ 90 bytes em media.
        let avg = total / 100;
        assert!((40..=MAX_OPUS_BYTES).contains(&avg), "media {avg} bytes, maximo {max}");
        assert!(enc.encode(&[0i16; 100], &mut buf).is_err());
    }

    #[test]
    fn dred_reconstroi_frames_perdidos() {
        let mut enc = Encoder::new(EncoderConfig::default()).unwrap();
        let mut dec = Decoder::new().unwrap();
        let mut pkts = Vec::new();
        let mut buf = [0u8; 400];
        for f in 0..60 {
            let n = enc.encode(&speech_like(f), &mut buf).unwrap();
            pkts.push(buf[..n].to_vec());
        }
        let mut out = [0i16; FRAME_SAMPLES];
        for p in &pkts[..40] {
            dec.decode(p, &mut out).unwrap();
        }
        // Perdem-se os frames 40, 41 e 42; chega o 43: o DRED dele cobre os tres.
        let covered = dec.parse_dred(&pkts[43], 3);
        assert!(covered >= 3, "DRED cobre {covered} frames");
        let mut recovered = 0;
        for back in (1..=3).rev() {
            if dec.decode_dred(back, &mut out).unwrap() {
                recovered += 1;
                assert!(energy(&out) > 1e3, "frame reconstruido em silencio");
            }
        }
        assert_eq!(recovered, 3);
        assert_eq!(dec.decode(&pkts[43], &mut out).unwrap(), FRAME_SAMPLES);
    }

    #[test]
    fn plc_quando_nao_ha_nada() {
        let mut enc = Encoder::new(EncoderConfig::default()).unwrap();
        let mut dec = Decoder::new().unwrap();
        let mut buf = [0u8; 400];
        let mut out = [0i16; FRAME_SAMPLES];
        for f in 0..20 {
            let n = enc.encode(&speech_like(f), &mut buf).unwrap();
            dec.decode(&buf[..n], &mut out).unwrap();
        }
        assert_eq!(dec.conceal(&mut out).unwrap(), FRAME_SAMPLES);
        // Sem `parse_dred` com sucesso, `decode_dred` faz PLC.
        assert!(!dec.decode_dred(1, &mut out).unwrap());
    }
}
