//! secure_messenger_core
//!
//! Nucleo criptografico do mensageiro. Esta biblioteca e consumida pelas
//! apps Android/iOS/Desktop via FFI (ver docs/architecture-decisions/
//! 0002-uniffi-ffi.md, a escrever). NENHUMA logica criptografica deve viver
//! na camada de UI - tudo passa por aqui, para que auditorias externas
//! possam focar-se num unico lugar.
//!
//! Estado atual (v0.2): X3DH classico + assinatura da signed pre-key +
//! one-time pre-keys + Double Ratchet, tudo compilado e testado com
//! `cargo test` (rustc 1.75). PQXDH (modulo pqxdh.rs) existe mas fica
//! atras da feature "pq" - requer rustc >= 1.81 (ver Cargo.toml).
//! Ver docs/protocol-spec.md para o que ainda falta antes de producao real
//! (transporte, sealed sender, armazenamento persistente).

pub mod primitives;
pub mod x3dh;
pub mod ratchet;

#[cfg(feature = "pq")]
pub mod pqxdh;

pub use primitives::{CryptoError, DhKeyPair, SigningKeyPair};
pub use ratchet::{EncryptedMessage, RatchetError, RatchetState};
pub use x3dh::{sign_pre_key, x3dh_initiate, x3dh_respond, PreKeyBundle, X3dhError, X3dhInitResult};
