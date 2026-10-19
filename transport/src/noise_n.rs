//! Noise_N: uma mensagem cifrada para a chave estatica de quem recebe, sem resposta.
//!
//! Usado no pedido para entrar numa sala de chamada (ver `qt_core::call`): o
//! telemovel cifra para a chave estatica do relay (a mesma que a app ja tem fixada,
//! o "pin"), sem se identificar. So o relay o consegue ler; quem espia a rede nao.
//! O prologo separa este uso da chave dos outros (o Noise_XX das mensagens).
//!
//! Formato (o do Noise): [32 chave efemera][conteudo cifrado + 16 tag].

use snow::Builder;

use crate::noise_session::NoiseError;

const PATTERN: &str = "Noise_N_25519_ChaChaPoly_SHA256";
/// Bytes a mais que o conteudo (chave efemera + tag).
pub const NOISE_N_OVERHEAD: usize = 32 + 16;

/// Cifra `payload` para `remote_static_public` (32 bytes).
pub fn noise_n_seal(remote_static_public: &[u8], prologue: &[u8], payload: &[u8]) -> Result<Vec<u8>, NoiseError> {
    let mut hs = Builder::new(PATTERN.parse()?)
        .remote_public_key(remote_static_public)
        .prologue(prologue)
        .build_initiator()?;
    let mut out = vec![0u8; payload.len() + NOISE_N_OVERHEAD];
    let n = hs.write_message(payload, &mut out)?;
    out.truncate(n);
    Ok(out)
}

/// Decifra com a chave estatica privada de quem recebe.
pub fn noise_n_open(local_static_private: &[u8], prologue: &[u8], message: &[u8]) -> Result<Vec<u8>, NoiseError> {
    if message.len() < NOISE_N_OVERHEAD {
        return Err(NoiseError::Protocol(snow::Error::Input));
    }
    let mut hs = Builder::new(PATTERN.parse()?)
        .local_private_key(local_static_private)
        .prologue(prologue)
        .build_responder()?;
    let mut out = vec![0u8; message.len()];
    let n = hs.read_message(message, &mut out)?;
    out.truncate(n);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate_static_keypair;

    #[test]
    fn ida_e_volta() {
        let k = generate_static_keypair().unwrap();
        let m = noise_n_seal(&k.public, b"p", b"ola relay").unwrap();
        assert_eq!(m.len(), 9 + NOISE_N_OVERHEAD);
        assert_eq!(noise_n_open(&k.private, b"p", &m).unwrap(), b"ola relay");
        // Cada pedido usa uma chave efemera nova: nunca sai igual.
        assert_ne!(m, noise_n_seal(&k.public, b"p", b"ola relay").unwrap());
    }

    #[test]
    fn chave_ou_prologo_errados_falham() {
        let (k, outra) = (generate_static_keypair().unwrap(), generate_static_keypair().unwrap());
        let m = noise_n_seal(&k.public, b"p", b"x").unwrap();
        assert!(noise_n_open(&outra.private, b"p", &m).is_err());
        assert!(noise_n_open(&k.private, b"q", &m).is_err());
        let mut t = m.clone();
        t[40] ^= 1;
        assert!(noise_n_open(&k.private, b"p", &t).is_err());
        assert!(noise_n_open(&k.private, b"p", &m[..10]).is_err());
    }
}
