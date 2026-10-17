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
    /// PRIMEIRA mensagem de qualquer ligacao. `signature` e uma assinatura
    /// Ed25519 (chave `signing_public`) sobre `AUTH_CONTEXT || hash do
    /// handshake Noise` - prova posse da chave privada e liga a prova a ESTA
    /// sessao (sem replay entre ligacoes). O `user_id` autenticado e
    /// `hex(signing_public)` (ver `user_id_from_signing_public`).
    Authenticate { signing_public: Vec<u8>, signature: Vec<u8> },

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
    ///
    /// Entrega AT-LEAST-ONCE: as mensagens NAO sao apagadas ao serem lidas;
    /// so depois de `AckMessages`. Devolve no maximo uma "pagina" que cabe
    /// num frame Noise; repetir (fetch -> processar -> ack) ate vir vazio.
    FetchMessages { user_id: UserId },

    /// Como `FetchMessages`, mas, se a fila estiver vazia, a resposta fica
    /// pendente ate chegar uma mensagem para este utilizador (ou ate
    /// `timeout_secs`, no maximo `waiters::MAX_WAIT_SECS`). Resposta:
    /// `Messages` (vazia se o tempo acabou). Entrega imediata sem o cliente
    /// ter de perguntar repetidamente; o cliente usa uma ligacao propria para
    /// isto, porque a ligacao fica ocupada durante a espera. Relays antigos
    /// respondem `Error` (pedido desconhecido): o cliente volta a `FetchMessages`
    /// periodico.
    WaitMessages { user_id: UserId, timeout_secs: u64 },

    /// Confirma que o cliente processou (e persistiu) estas mensagens; so
    /// agora o relay as apaga. Duplicados na entrega sao apanhados pelo
    /// cliente (replay protection no Double Ratchet/storage).
    AckMessages { user_id: UserId, ids: Vec<u64> },

    // ----- Anexos (ver blobs.rs). Os bytes vao JA CIFRADOS pelo cliente. -----
    /// Reserva um ficheiro de `size` bytes. Resposta: `BlobCreated`.
    BlobCreate { size: u64 },
    /// Escreve um bloco (base64, ate `BLOB_CHUNK` bytes) a partir de `offset`.
    /// So quem tem o `token` de `BlobCreated` escreve. Resposta: `BlobStatus`.
    BlobPut { blob_id: String, token: String, offset: u64, data: String },
    /// Quantos bytes ja chegaram. Resposta: `BlobStatus` ou `BlobNotFound`.
    BlobInfo { blob_id: String },
    /// Le um bloco de um ficheiro completo. Resposta: `BlobData` ou `BlobNotFound`.
    BlobGet { blob_id: String, offset: u64 },
    /// Apaga o ficheiro (o destinatario ja o descarregou). Resposta: `Ack`.
    BlobDelete { blob_id: String },

    /// Apaga a conta do proprio utilizador no relay: bundle de pre-keys, fila de
    /// mensagens e contagem diaria de anexos. Durante `store::DELETED_TTL_SECONDS`,
    /// quem tentar enviar-lhe uma mensagem recebe `Error` com
    /// `RECIPIENT_DELETED`. Resposta: `Ack`. Relays antigos respondem `Error`
    /// (pedido desconhecido).
    DeleteAccount { user_id: UserId },
}

/// Motivo do `Error` a um `SendMessage` para quem apagou a conta.
pub const RECIPIENT_DELETED: &str = "o destinatario apagou a conta";

/// Contexto de dominio da assinatura de autenticacao.
pub const AUTH_CONTEXT: &[u8] = b"QT-RELAY-AUTH-v1";

pub fn user_id_from_signing_public(signing_public: &[u8]) -> UserId {
    signing_public.iter().map(|b| format!("{:02x}", b)).collect()
}

/// `user_id` valido = 64 caracteres hexadecimais minusculos (32 bytes).
/// Impede ids arbitrarios/gigantes nas chaves do Redis.
pub fn is_valid_user_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Uma mensagem entregue da fila. `sealed_from` so o destinatario consegue
/// abrir (ver `core::sealed_sender`) - o servidor nunca soube quem enviou.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeliveredMessage {
    pub id: u64,
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
    BlobCreated { blob_id: String, token: String },
    BlobStatus { size: u64, received: u64, complete: bool },
    /// `data` em base64.
    BlobData { offset: u64, size: u64, data: String },
    BlobNotFound,
}

pub fn serialize_client_message(msg: &ClientMessage) -> Vec<u8> {
    serde_json::to_vec(msg).unwrap_or_default()
}

pub fn deserialize_client_message(bytes: &[u8]) -> Result<ClientMessage, serde_json::Error> {
    serde_json::from_slice(bytes)
}

pub fn serialize_server_message(msg: &ServerMessage) -> Vec<u8> {
    serde_json::to_vec(msg).unwrap_or_default()
}

pub fn deserialize_server_message(bytes: &[u8]) -> Result<ServerMessage, serde_json::Error> {
    serde_json::from_slice(bytes)
}
