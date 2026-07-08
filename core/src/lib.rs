//! secure_messenger_core
//!
//! Nucleo criptografico do mensageiro. Esta biblioteca e consumida pelas
//! apps Android/iOS/Desktop via FFI (ver docs/architecture-decisions/
//! 0002-uniffi-ffi.md, a escrever). NENHUMA logica criptografica deve viver
//! na camada de UI - tudo passa por aqui, para que auditorias externas
//! possam focar-se num unico lugar.
//!
//! Estado atual: prova de conceito didatica (X3DH classico + Double Ratchet).
//! Ver TODOs em x3dh.rs para o que falta antes de uso em producao:
//! PQXDH (Kyber), assinatura da signed pre-key, one-time pre-keys.

pub mod primitives;
pub mod x3dh;
pub mod ratchet;

pub use primitives::{CryptoError, DhKeyPair};
pub use ratchet::{EncryptedMessage, RatchetError, RatchetState};
pub use x3dh::{x3dh_initiate, x3dh_respond, PreKeyBundle, X3dhInitResult};
