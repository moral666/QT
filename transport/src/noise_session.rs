//! Wrapper sobre o Noise Protocol Framework (crate `snow`).
//!
//! Este canal Noise protege o TRANSPORTE (cliente<->servidor/relay) - e uma
//! camada de seguranca separada e adicional ao E2EE de aplicacao que ja
//! existe em `core/` (X3DH + Double Ratchet). Mesmo que este canal seja
//! comprometido, o conteudo das mensagens continua protegido pelo Double
//! Ratchet; esta camada existe para autenticar a ligacao e ofuscar
//! metadados de transporte (evita que um observador de rede identifique
//! trivialmente o protocolo de aplicacao por cima).
//!
//! Padrao escolhido: Noise_XX_25519_ChaChaPoly_SHA256 - permite autenticacao
//! mutua sem que as partes precisem de conhecer antecipadamente a chave
//! estatica uma da outra (adequado para a primeira ligacao cliente-servidor;
//! ligacoes servidor-servidor de federacao podem usar Noise_IK, que exige
//! menos round-trips quando a chave do destino ja e conhecida - TODO futuro).

use snow::{Builder, HandshakeState, TransportState};
use thiserror::Error;

const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_SHA256";

/// Tamanho maximo de uma mensagem individual do Noise (limite do protocolo).
const MAX_MESSAGE_LEN: usize = 65535;

#[derive(Debug, Error)]
pub enum NoiseError {
    #[error("erro no protocolo Noise: {0}")]
    Protocol(#[from] snow::Error),
    #[error("handshake ainda nao terminou - chame handshake_step ate is_finished() devolver true")]
    HandshakeNotFinished,
    #[error("handshake ja terminou - use encrypt/decrypt em vez de handshake_step")]
    HandshakeAlreadyFinished,
}

/// Par de chaves estaticas Noise, geradas uma vez por instalacao (nao por
/// sessao). Persistido localmente pelo cliente/servidor entre reinicios,
/// para que o par remoto possa reconhecer a mesma identidade de transporte
/// em ligacoes futuras (pinning implicito, similar a um "known_hosts").
pub struct NoiseStaticKeyPair {
    pub private: Vec<u8>,
    pub public: Vec<u8>,
}

pub fn generate_static_keypair() -> Result<NoiseStaticKeyPair, NoiseError> {
    let builder = Builder::new(NOISE_PATTERN.parse()?);
    let keypair = builder.generate_keypair()?;
    Ok(NoiseStaticKeyPair { private: keypair.private, public: keypair.public })
}

/// Estado de uma sessao Noise em curso de handshake. Depois de
/// `is_finished()` devolver true, converte-se em `NoiseTransport` via
/// `into_transport()`.
pub struct NoiseHandshake {
    state: HandshakeState,
}

impl NoiseHandshake {
    pub fn new_initiator(local_static_private: &[u8]) -> Result<Self, NoiseError> {
        let builder = Builder::new(NOISE_PATTERN.parse()?);
        let state = builder
            .local_private_key(local_static_private)
            .build_initiator()?;
        Ok(Self { state })
    }

    pub fn new_responder(local_static_private: &[u8]) -> Result<Self, NoiseError> {
        let builder = Builder::new(NOISE_PATTERN.parse()?);
        let state = builder
            .local_private_key(local_static_private)
            .build_responder()?;
        Ok(Self { state })
    }

    pub fn is_finished(&self) -> bool {
        self.state.is_handshake_finished()
    }

    /// Produz a proxima mensagem de handshake a enviar ao par remoto.
    /// Chamar alternadamente entre os dois lados ate `is_finished()`.
    pub fn write_step(&mut self) -> Result<Vec<u8>, NoiseError> {
        let mut buf = vec![0u8; MAX_MESSAGE_LEN];
        let len = self.state.write_message(&[], &mut buf)?;
        buf.truncate(len);
        Ok(buf)
    }

    /// Processa uma mensagem de handshake recebida do par remoto.
    pub fn read_step(&mut self, message: &[u8]) -> Result<(), NoiseError> {
        let mut buf = vec![0u8; MAX_MESSAGE_LEN];
        self.state.read_message(message, &mut buf)?;
        Ok(())
    }

    /// Chave publica estatica do par remoto, disponivel apos o handshake
    /// avancar o suficiente para a ter recebido (padrao XX: apos o 2o passo).
    /// Util para "pinning": comparar com uma chave conhecida anteriormente.
    pub fn remote_static_public_key(&self) -> Option<Vec<u8>> {
        self.state.get_remote_static().map(|k| k.to_vec())
    }

    /// Converte o handshake terminado no estado de transporte (cifra/decifra
    /// de mensagens de aplicacao a partir daqui).
    pub fn into_transport(self) -> Result<NoiseTransport, NoiseError> {
        if !self.state.is_handshake_finished() {
            return Err(NoiseError::HandshakeNotFinished);
        }
        let transport = self.state.into_transport_mode()?;
        Ok(NoiseTransport { transport })
    }
}

/// Canal Noise apos o handshake, pronto para cifrar/decifrar mensagens de
/// aplicacao (que, nesta arquitetura, ja vem cifradas pelo Double Ratchet -
/// isto e uma segunda camada, "defesa em profundidade" do transporte).
pub struct NoiseTransport {
    transport: TransportState,
}

impl NoiseTransport {
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, NoiseError> {
        let mut buf = vec![0u8; plaintext.len() + 16]; // +16 = tag do AEAD
        let len = self.transport.write_message(plaintext, &mut buf)?;
        buf.truncate(len);
        Ok(buf)
    }

    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, NoiseError> {
        let mut buf = vec![0u8; ciphertext.len()];
        let len = self.transport.read_message(ciphertext, &mut buf)?;
        buf.truncate(len);
        Ok(buf)
    }
}
