//! Formato de serializacao do `PreKeyBundle` (de `core::x3dh`) para viajar
//! como bytes opacos atraves do servidor/relay.
//!
//! Fica no CLI (nao em `core/`) deliberadamente: `core/` deve permanecer
//! livre de dependencias de serializacao, para manter a superficie de
//! auditoria minima (ver README/CONTRIBUTING). Layout fixo e documentado
//! aqui, simples o suficiente para nao precisar de serde/bincode:
//!
//! [32 bytes identity_key] [32 bytes identity_signing_key (Ed25519)]
//! [32 bytes signed_pre_key] [64 bytes signed_pre_key_signature]
//! [1 byte: 1 se houver one_time_pre_key, 0 caso contrario]
//! [32 bytes one_time_pre_key, so presente se o byte anterior for 1]

use ed25519_dalek::{Signature, VerifyingKey};
use secure_messenger_core::x3dh::PreKeyBundle;
use x25519_dalek::PublicKey;

#[derive(Debug)]
pub struct WireFormatError(pub String);

pub fn serialize_bundle(bundle: &PreKeyBundle) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + 32 + 32 + 64 + 1 + 32);
    out.extend_from_slice(bundle.identity_key.as_bytes());
    out.extend_from_slice(bundle.identity_signing_key.as_bytes());
    out.extend_from_slice(bundle.signed_pre_key.as_bytes());
    out.extend_from_slice(&bundle.signed_pre_key_signature.to_bytes());
    match &bundle.one_time_pre_key {
        Some(otk) => {
            out.push(1);
            out.extend_from_slice(otk.as_bytes());
        }
        None => out.push(0),
    }
    out
}

pub fn deserialize_bundle(bytes: &[u8]) -> Result<PreKeyBundle, WireFormatError> {
    if bytes.len() < 32 + 32 + 32 + 64 + 1 {
        return Err(WireFormatError("bundle demasiado curto".into()));
    }

    let identity_key = read_public_key(&bytes[0..32])?;
    let identity_signing_key = VerifyingKey::from_bytes(bytes[32..64].try_into().unwrap())
        .map_err(|e| WireFormatError(format!("chave de assinatura invalida: {e}")))?;
    let signed_pre_key = read_public_key(&bytes[64..96])?;
    let signed_pre_key_signature = Signature::from_bytes(bytes[96..160].try_into().unwrap());

    let has_otk = bytes[160];
    let one_time_pre_key = if has_otk == 1 {
        if bytes.len() < 161 + 32 {
            return Err(WireFormatError("bundle indica one-time pre-key mas faltam bytes".into()));
        }
        Some(read_public_key(&bytes[161..193])?)
    } else {
        None
    };

    Ok(PreKeyBundle {
        identity_key,
        identity_signing_key,
        signed_pre_key,
        signed_pre_key_signature,
        one_time_pre_key,
    })
}

fn read_public_key(bytes: &[u8]) -> Result<PublicKey, WireFormatError> {
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| WireFormatError("chave publica com tamanho invalido".into()))?;
    Ok(PublicKey::from(arr))
}
