//! Protocolo de aplicacao trocado DENTRO do canal Noise (ver `transport/`).
//! O servidor nunca ve isto em texto plano fora do canal Noise, e nunca ve o
//! CONTEUDO das mensagens (`ciphertext` aqui e sempre o resultado do Double
//! Ratchet em `core/` - o servidor so guarda e reencaminha bytes opacos).
//!
//! Serializado com JSON por simplicidade/legibilidade nesta fase do
//! projeto. Antes de producao, considerar um formato binario mais compacto
//! (bincode, protobuf) para reduzir overhead de banda em mobile.

use serde::{Deserialize, Serialize};

/// Identificador de utilizador. Nesta fase, uma string opaca (ex.: hash da
/// chave de identidade publica - ver core/). O servidor nunca sabe o
/// telefone/email de ninguem, apenas este identificador.
pub type UserId = String;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ClientMessage {
    /// Publica o bundle de pre-keys publicas do proprio utilizador, para
    /// que outros possam iniciar uma sessao X3DH/PQXDH mesmo offline.
    /// `bundle_bytes` e uma serializacao opaca do `PreKeyBundle` de
    /// `core/` (formato de serializacao ainda a definir - TODO).
    RegisterPreKeyBundle { user_id: UserId, bundle_bytes: Vec<u8> },

    /// Pede o bundle de pre-keys publicado por outro utilizador.
    FetchPreKeyBundle { user_id: UserId },

    /// Envia uma mensagem ja cifrada (Double Ratchet) para outro
    /// utilizador. Fica em fila no servidor ate ele se ligar.
    ///
    /// `sealed_from` e um envelope produzido por
    /// `core::sealed_sender::seal_sender_identity` - o servidor NAO
    /// consegue ler quem enviou (so o destinatario, com a sua chave
    /// privada de identidade, consegue abrir o envelope). O servidor
    /// continua a saber `to` (precisa disso para rotear para a fila certa).
    SendMessage { to: UserId, sealed_from: Vec<u8>, ciphertext: Vec<u8> },

    /// Pede todas as mensagens em fila destinadas ao proprio utilizador
    /// (chamado logo apos autenticar-se/ligar-se).
    FetchMessages { user_id: UserId },
}

/// Uma mensagem entregue da fila. `sealed_from` so o destinatario consegue
/// abrir (ver `core::sealed_sender`) - o servidor nunca soube quem enviou.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeliveredMessage {
    pub sealed_from: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ServerMessage {
    Ack,
    PreKeyBundle { bundle_bytes: Vec<u8> },
    PreKeyBundleNotFound,
    Messages { messages: Vec<DeliveredMessage> },
    Error { reason: String },
}

pub fn serialize_client_message(msg: &ClientMessage) -> Vec<u8> {
    serde_json::to_vec(msg).expect("serializacao de ClientMessage nao deveria falhar")
}

pub fn deserialize_client_message(bytes: &[u8]) -> Result<ClientMessage, serde_json::Error> {
    serde_json::from_slice(bytes)
}

pub fn serialize_server_message(msg: &ServerMessage) -> Vec<u8> {
    serde_json::to_vec(msg).expect("serializacao de ServerMessage nao deveria falhar")
}

pub fn deserialize_server_message(bytes: &[u8]) -> Result<ServerMessage, serde_json::Error> {
    serde_json::from_slice(bytes)
}
