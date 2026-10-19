//! qt-call-demo: testar as chamadas de voz no PC, sem telemoveis.
//!
//! MODO ECO: dois motores de chamada neste PC (o mesmo codigo que a app vai usar).
//! O microfone entra no motor A, que envia para o relay das chamadas (Estocolmo); o
//! relay entrega ao motor B, que toca nos auscultadores. Ouves-te com o atraso real
//! da rede, do jitter buffer e do Opus.
//!
//!   cargo run --release -- eco                 # microfone -> relay -> auscultadores
//!   cargo run --release -- eco --perda 15      # deita fora 15% dos pacotes (DRED/PLC)
//!   cargo run --release -- eco --ws            # usa a reserva wss://.../call
//!   cargo run --release -- eco --tom           # som de teste em vez do microfone
//!
//! Outras opcoes: --relay <nome> (relay.sealer.pt), --pin <hex> (chave do relay),
//! --segundos <n> (60), --debito <bit/s> (36000).
//!
//! USA AUSCULTADORES: com colunas, o som volta a entrar no microfone (eco). No PC nao
//! ha cancelamento de eco; nos telemoveis vai haver o do sistema (fase 4).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use qt_call::codec::{FRAME_SAMPLES, SAMPLE_RATE};
use qt_call::engine::{join_packet, CallEngine, EngineConfig, EngineStats, RelayTarget, Transport};
use qt_core::call::{CallKeys, MediaSession, Role, RoomKeys};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RELAY: &str = "relay.sealer.pt";
const PIN: &str = "f018d46b67d2437571cefc86827dec0dc3b0fd586bd22c664d3839f6a1c67f1a";

struct Opts {
    perda: u32,
    ws: bool,
    tom: bool,
    relay: String,
    pin: String,
    segundos: u64,
    debito: i32,
}

fn fatal(msg: &str) -> ! {
    eprintln!("erro: {msg}");
    std::process::exit(1);
}

fn parse_args() -> Opts {
    let mut o = Opts { perda: 0, ws: false, tom: false, relay: RELAY.into(), pin: PIN.into(), segundos: 60, debito: 36_000 };
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("eco") => {}
        _ => {
            eprintln!("uso: qt-call-demo eco [--perda N] [--ws] [--tom] [--relay NOME] [--pin HEX] [--segundos N] [--debito BIT/S]");
            std::process::exit(2);
        }
    }
    while let Some(a) = args.next() {
        let mut val = |nome: &str| args.next().unwrap_or_else(|| fatal(&format!("falta o valor de {nome}")));
        match a.as_str() {
            "--perda" => o.perda = val("--perda").parse().unwrap_or_else(|_| fatal("--perda: numero de 0 a 100")),
            "--ws" => o.ws = true,
            "--tom" => o.tom = true,
            "--relay" => o.relay = val("--relay"),
            "--pin" => o.pin = val("--pin"),
            "--segundos" => o.segundos = val("--segundos").parse().unwrap_or_else(|_| fatal("--segundos: numero")),
            "--debito" => o.debito = val("--debito").parse().unwrap_or_else(|_| fatal("--debito: bit/s")),
            other => fatal(&format!("opcao desconhecida: {other}")),
        }
    }
    o
}

fn hex(s: &str) -> Vec<u8> {
    if s.len() != 64 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        fatal("--pin: 64 caracteres hexadecimais");
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn rnd<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).expect("aleatorio");
    b
}

/// "Voz" de teste: harmonicos com silabas (para --tom).
fn tom(frame: usize) -> Vec<i16> {
    (0..FRAME_SAMPLES)
        .map(|i| {
            let t = (frame * FRAME_SAMPLES + i) as f64 / SAMPLE_RATE as f64;
            let f0 = 140.0 + 30.0 * (t * 2.0 * std::f64::consts::PI * 0.7).sin();
            let env = (0.5 + 0.5 * (t * 2.0 * std::f64::consts::PI * 3.0).sin()).powi(2);
            let mut v = 0.0;
            for h in 1..12 {
                v += (t * 2.0 * std::f64::consts::PI * f0 * h as f64).sin() / h as f64;
            }
            (v * env * 5000.0) as i16
        })
        .collect()
}

fn f32_to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0) as i16
}

fn main() {
    let o = parse_args();
    let pin = hex(&o.pin);

    // Uma chamada "ja atendida": segredos aleatorios, como os da oferta e da resposta.
    let (offer, answer, id) = (rnd::<32>(), rnd::<32>(), rnd::<16>());
    let room = RoomKeys::derive(&offer, &id);
    let media = |role| MediaSession::new(CallKeys::derive(&offer, &answer, &id, role), 128).expect("sessao");
    let target = RelayTarget { udp: format!("{}:443", o.relay), ws_url: Some(format!("wss://{}/call", o.relay)) };
    let mut cfg_a = EngineConfig::new(target.clone());
    cfg_a.encoder.bitrate = o.debito;
    cfg_a.force_ws = o.ws;
    let mut cfg_b = EngineConfig::new(target);
    cfg_b.force_ws = o.ws;
    cfg_b.simulated_loss_percent = o.perda.min(100);

    println!("A ligar ao relay das chamadas {} ({}) ...", o.relay, if o.ws { "WebSocket" } else { "UDP, com reserva WebSocket" });
    let a = Arc::new(
        CallEngine::start(cfg_a, media(Role::Caller), join_packet(&pin, Role::Caller, &room).expect("pin"))
            .unwrap_or_else(|e| fatal(&format!("motor A: {e}"))),
    );
    let b = Arc::new(
        CallEngine::start(cfg_b, media(Role::Callee), join_packet(&pin, Role::Callee, &room).expect("pin"))
            .unwrap_or_else(|e| fatal(&format!("motor B: {e}"))),
    );
    let t = Instant::now();
    while !(a.stats().joined && b.stats().joined) {
        if t.elapsed() > Duration::from_secs(6) {
            fatal(&format!("sem resposta do relay em 6 s (A: {:?}, B: {:?})", a.stats().transport, b.stats().transport));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let st = a.stats();
    println!(
        "Na sala por {} em {} ms. Ida e volta ate ao relay: {} ms.",
        transport(st.transport),
        t.elapsed().as_millis(),
        st.relay_rtt_ms.map_or("?".into(), |r| r.to_string())
    );

    // ---------- audio do PC ----------
    let host = cpal::default_host();
    let out_dev = host.default_output_device().unwrap_or_else(|| fatal("sem dispositivo de saida de audio"));
    let out_cfg = stream_config(out_dev.default_output_config().unwrap_or_else(|e| fatal(&format!("saida: {e}"))));
    let out_ch = out_cfg.0.channels as usize;
    let pending: Arc<Mutex<VecDeque<i16>>> = Arc::new(Mutex::new(VecDeque::new()));
    let fill = {
        let (b, pending) = (b.clone(), pending.clone());
        move |frames: usize| -> Vec<i16> {
            let mut p = pending.lock().unwrap();
            while p.len() < frames {
                let mut buf = vec![0i16; FRAME_SAMPLES];
                let _ = b.pull_playback(&mut buf);
                p.extend(buf);
            }
            p.drain(..frames).collect()
        }
    };
    let err = |e| eprintln!("audio: {e}");
    let out_stream = match out_cfg.1 {
        cpal::SampleFormat::F32 => out_dev.build_output_stream(
            &out_cfg.0,
            move |data: &mut [f32], _| {
                let mono = fill(data.len() / out_ch);
                for (frame, s) in data.chunks_mut(out_ch).zip(mono) {
                    frame.fill(s as f32 / 32768.0);
                }
            },
            err,
            None,
        ),
        cpal::SampleFormat::I16 => out_dev.build_output_stream(
            &out_cfg.0,
            move |data: &mut [i16], _| {
                let mono = fill(data.len() / out_ch);
                for (frame, s) in data.chunks_mut(out_ch).zip(mono) {
                    frame.fill(s);
                }
            },
            err,
            None,
        ),
        f => fatal(&format!("formato de saida nao suportado: {f:?}")),
    }
    .unwrap_or_else(|e| fatal(&format!("saida de audio a 48 kHz: {e}")));

    // Entrada: microfone, ou o som de teste a 20 ms (thread propria).
    let mut _in_stream = None;
    if o.tom {
        let a = a.clone();
        std::thread::spawn(move || {
            let t0 = Instant::now();
            for i in 0.. {
                let _ = a.push_capture(&tom(i));
                let next = t0 + Duration::from_millis(20 * (i as u64 + 1));
                if let Some(d) = next.checked_duration_since(Instant::now()) {
                    std::thread::sleep(d);
                }
            }
        });
    } else {
        let in_dev = host.default_input_device().unwrap_or_else(|| fatal("sem microfone (usa --tom para um som de teste)"));
        let in_cfg = stream_config(in_dev.default_input_config().unwrap_or_else(|e| fatal(&format!("microfone: {e}"))));
        let in_ch = in_cfg.0.channels as usize;
        let acc: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::with_capacity(FRAME_SAMPLES * 2)));
        let push = {
            let a = a.clone();
            move |mono: &mut dyn Iterator<Item = i16>| {
                let mut acc = acc.lock().unwrap();
                acc.extend(mono);
                while acc.len() >= FRAME_SAMPLES {
                    let frame: Vec<i16> = acc.drain(..FRAME_SAMPLES).collect();
                    let _ = a.push_capture(&frame);
                }
            }
        };
        let s = match in_cfg.1 {
            cpal::SampleFormat::F32 => in_dev.build_input_stream(
                &in_cfg.0,
                move |data: &[f32], _| {
                    push(&mut data.chunks(in_ch).map(|c| f32_to_i16(c.iter().sum::<f32>() / in_ch as f32)));
                },
                err,
                None,
            ),
            cpal::SampleFormat::I16 => in_dev.build_input_stream(
                &in_cfg.0,
                move |data: &[i16], _| {
                    push(&mut data.chunks(in_ch).map(|c| (c.iter().map(|&x| x as i32).sum::<i32>() / in_ch as i32) as i16));
                },
                err,
                None,
            ),
            f => fatal(&format!("formato do microfone nao suportado: {f:?}")),
        }
        .unwrap_or_else(|e| fatal(&format!("microfone a 48 kHz: {e}")));
        s.play().unwrap_or_else(|e| fatal(&format!("microfone: {e}")));
        _in_stream = Some(s);
    }
    out_stream.play().unwrap_or_else(|e| fatal(&format!("saida: {e}")));

    println!(
        "\nA falar contigo mesmo{} durante {} s. USA AUSCULTADORES. Enter para terminar.\n",
        if o.perda > 0 { format!(", com {}% de pacotes perdidos", o.perda) } else { String::new() },
        o.segundos
    );
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let done = done.clone();
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::stdin().read_line(&mut s);
            done.store(true, std::sync::atomic::Ordering::Relaxed);
        });
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(o.segundos) && !done.load(std::sync::atomic::Ordering::Relaxed) {
        std::thread::sleep(Duration::from_secs(2));
        println!("{}", line(&a.stats(), &b.stats()));
    }
    drop(out_stream);
    drop(_in_stream);
    println!("\nResumo: {}", line(&a.stats(), &b.stats()));
}

fn stream_config(c: cpal::SupportedStreamConfig) -> (cpal::StreamConfig, cpal::SampleFormat) {
    // Pede sempre 48 kHz (o Opus trabalha a 48 kHz; o PipeWire/Pulse converte se for preciso).
    let cfg = cpal::StreamConfig { channels: c.channels(), sample_rate: cpal::SampleRate(SAMPLE_RATE as u32), buffer_size: cpal::BufferSize::Default };
    (cfg, c.sample_format())
}

fn transport(t: Transport) -> &'static str {
    match t {
        Transport::Connecting => "a ligar",
        Transport::Udp => "UDP",
        Transport::WebSocket => "WebSocket",
    }
}

fn line(a: &EngineStats, b: &EngineStats) -> String {
    let j = &b.jitter;
    let rtt = a.relay_rtt_ms.unwrap_or(0);
    // Boca -> ouvido (aproximado): 20 ms do frame + ~7 do Opus + rede (no eco, ida e
    // volta ao relay) + jitter buffer + ~30 ms das placas de som.
    let total = 20 + 7 + rtt + j.depth_ms + 30;
    format!(
        "{} | relay {} ms | jitter buffer {} ms (alvo {}, variacao {} ms) | recebidos {} | perdidos {} -> DRED {}, PLC {} | esperas {} | saltos {} | atrasados {} | Opus {} B | boca->ouvido ~{} ms",
        transport(b.transport),
        rtt,
        j.depth_ms,
        j.target_ms,
        j.jitter_ms,
        b.received,
        b.simulated_drops + j.lost,
        b.dred_frames,
        b.plc_frames,
        j.holds,
        j.skips,
        j.late,
        a.avg_opus_bytes,
        total
    )
}
