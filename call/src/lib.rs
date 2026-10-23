//! qt_call: o motor das chamadas de voz (fase 3).
//!
//! Microfone -> Opus (`codec`) -> cifra (`qt_core::call`) -> relay das chamadas
//! (`link`) -> decifra -> `jitter` -> Opus (DRED / PLC neural) -> altifalante.
//! A app (Kotlin, fase 4) so entrega 20 ms de microfone e pede 20 ms para tocar.

pub mod codec;
pub mod jitter;
pub mod engine;
pub mod fichas;
pub mod video;
