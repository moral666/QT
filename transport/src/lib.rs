//! qt_transport
//!
//! Camada de transporte: Noise Protocol Framework sobre WebSocket.
//! Agnostica do conteudo - so ve bytes ja cifrados pelo `core/` (Double
//! Ratchet). Ver docs/protocol-spec.md secao 3.

pub mod noise_n;
pub mod noise_session;
pub mod ws_transport;

pub use noise_n::{noise_n_open, noise_n_seal, NOISE_N_OVERHEAD};
pub use noise_session::{
    generate_static_keypair, static_keypair_from_private_bytes, NoiseError, NoiseHandshake,
    NoiseStaticKeyPair, NoiseTransport,
};
