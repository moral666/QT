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

/// Marca da campainha de uma chamada de voz (`ClientMessage::SendMessage::wake`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WakeKind {
    Call,
    CallEnd,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ClientMessage {
    /// PRIMEIRA mensagem de qualquer ligacao. `signature` e uma assinatura
    /// Ed25519 (chave `signing_public`) sobre `AUTH_CONTEXT || hash do
    /// handshake Noise` - prova posse da chave privada e liga a prova a ESTA
    /// sessao (sem replay entre ligacoes). O `user_id` autenticado e
    /// `hex(signing_public)` (ver `user_id_from_signing_public`).
    Authenticate { signing_public: Vec<u8>, signature: Vec<u8> },

    /// Publica o bundle de pre-keys publicas do proprio utilizador, para
    /// que outros possam iniciar uma sessao PQXDH mesmo offline (formato em
    /// `qt_core::identity`: v4, ou v3 das apps ate a 0.9.2).
    ///
    /// - `pow`: prova de trabalho da conta (`qt_core::pow`). Obrigatoria quando o
    ///   relay ainda nao tem bundle desta conta (conta nova ou expirada).
    /// - `one_time_pre_keys`: se vier, SUBSTITUI as one-time prekeys guardadas
    ///   (`[4 numero BE][32 publica]` cada, ate `MAX_OTPKS`). O relay entrega uma
    ///   por cada `FetchPreKeyBundle` e apaga-a.
    RegisterPreKeyBundle {
        user_id: UserId,
        bundle_bytes: Vec<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pow: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        one_time_pre_keys: Option<Vec<Vec<u8>>>,
    },

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
    ///
    /// `silent = true`: recibos, reacoes e avisos de presenca. A mensagem e entregue
    /// igual, mas o relay NAO toca a campainha (FCM) a uma app fechada. Ausente
    /// (clientes antigos) = `false`: toca como antes.
    SendMessage {
        to: UserId,
        sealed_from: Vec<u8>,
        ciphertext: Vec<u8>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        silent: bool,
        /// Chamadas de voz: oferta (`Call`) ou cancelada antes de atender (`CallEnd`). A
        /// campainha vai marcada, para uma app fechada com o bloqueio ativo (que nao
        /// consegue decifrar) tocar ou parar. Ausente = mensagem normal.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wake: Option<WakeKind>,
    },

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

    /// Notificacoes com a app fechada: o "endereco de campainha" (token do Firebase
    /// Cloud Messaging) do proprio utilizador. Quando lhe chega uma mensagem e nao ha
    /// nenhuma ligacao dele a espera (`WaitMessages`), o relay pede a Google para
    /// acordar o telemovel, com uma mensagem VAZIA (sem remetente nem conteudo).
    /// `token` vazio = deixar de receber. Guardado com `store::PUSH_TTL_SECONDS` e
    /// apagado com a conta. Resposta: `Ack`. Relays antigos respondem `Error`.
    RegisterPushToken { user_id: UserId, token: String },

    /// Denuncia de um contacto, ja cifrada pela app para a chave do operador
    /// (`qt_core::report`): o relay nao a consegue ler. Guardada `store::REPORT_TTL_SECONDS`
    /// SEM o ID de quem denunciou. Ate `store::MAX_REPORTS_PER_DAY` por conta e por dia.
    /// Resposta: `Ack`. Relays antigos respondem `Error` (pedido desconhecido).
    SubmitReport { sealed: Vec<u8> },

    // ----- Pedidos do OPERADOR: so a chave Ed25519 de /etc/qt/operator.pub. -----
    /// Denuncias guardadas com id maior do que `after`, ate encher uma pagina.
    /// Resposta: `Reports` (`more = true`: repetir com o ultimo id recebido).
    AdminListReports { after: u64 },
    /// Apaga uma denuncia (ja tratada). Resposta: `Ack`.
    AdminDeleteReport { id: u64 },
    /// Bane uma conta: apaga o que o relay guarda dela (bundle, fila, campainha) e
    /// passa a recusar a autenticacao e o registo. Fecha as ligacoes abertas. `Ack`.
    AdminBan { user_id: UserId },
    /// Levanta o banimento. Resposta: `Ack`.
    AdminUnban { user_id: UserId },
    /// Contas banidas. Resposta: `Bans`.
    AdminListBans,

    // ----- Copia de recuperacao (cifrada na app: ver `qt_core::recovery`). -----
    /// Uma parte da copia `lookup` (identificador derivado das 12 palavras, 64 hex). Envia-se
    /// `part` de 0 a `parts - 1`, todas com a mesma `generation` (16 hex); `data` em base64,
    /// ate `BACKUP_PART_MAX` bytes. Quando chega a ultima, a copia nova substitui a anterior
    /// (e a de outras palavras desta conta, se as houver). So contas registadas; so o dono
    /// da copia a substitui. Guardada 90 dias, renovados quando a app abre. Resposta: `Ack`.
    BackupPut { lookup: String, generation: String, part: u32, parts: u32, data: String },
    /// Uma parte da copia `lookup` (qualquer ligacao autenticada: num telemovel novo a app
    /// ainda nao tem a conta). Resposta: `BackupPart` ou `BackupNotFound`.
    BackupGet { lookup: String, part: u32 },
    /// Apaga a copia desta conta. Resposta: `Ack`.
    BackupDelete,

    // ----- Premium (desde a 0.9.20). -----
    /// Ate quando esta conta tem premium. Resposta: `Premium`.
    PremiumStatus,
    /// OPERADOR: da premium a uma conta ate `until_ms` (ms desde 1970); 0 tira. `Ack`.
    AdminGrantPremium { user_id: UserId, until_ms: u64 },
    /// OPERADOR: contas com premium. Resposta: `PremiumList`.
    AdminListPremium,
    /// OPERADOR (0.9.21): liga ou desliga o bloqueio premium para todos. Com ele ligado, o
    /// relay recusa anexos (`BlobCreate`) e chamadas (`SendMessage` com `wake: Call`) a contas
    /// sem premium e as apps fecham as funcoes pagas. `None` so pergunta. Resposta: `PremiumEnforce`.
    AdminPremiumEnforce { on: Option<bool> },

    // ----- Fichas anonimas do video pelo relay (0006, ver `qt_core::fichas`). -----
    /// Pede fichas do mes `epoch` (AAAAMM, UTC), CEGAS: `blinded` sao n elementos de 32 bytes
    /// (ristretto255) em base64, com n ate `qt_core::fichas::MAX_BATCH`. O relay so conta
    /// quantas deu a esta conta neste mes (ate ao teto: `ServerConfig::fichas_per_month`, ou o
    /// desta conta) e nunca guarda o pedido nem a resposta. So contas registadas.
    /// Resposta: `Fichas`. Relays antigos respondem `Error` (pedido desconhecido).
    IssueFichas { epoch: u32, blinded: String },
    /// OPERADOR: fichas por mes desta conta (quota propria); `None` volta
    /// ao valor do relay. Resposta: `Ack`.
    AdminSetFichasQuota { user_id: UserId, per_month: Option<u32> },
    /// OPERADOR (27/10/2026): quantas contas tem a app ligada ao relay agora (contas
    /// diferentes, nao ligacoes; sem o operador). So o numero, contado em memoria.
    /// Resposta: `Online`. Relays antigos respondem `Error`.
    AdminOnline,
}

/// Uma conta com premium (lista do operador).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct PremiumEntry {
    pub user_id: UserId,
    pub until_ms: u64,
}

/// Maior parte de uma copia de recuperacao (bytes, antes do base64).
pub const BACKUP_PART_MAX: usize = 40 * 1024;
/// Partes de uma copia, no maximo (ate 320 KB).
pub const BACKUP_PARTS_MAX: u32 = 8;
/// Motivo do `Error` a quem tenta substituir a copia de outra conta.
pub const BACKUP_NOT_OWNER: &str = "esta copia pertence a outra conta";

/// Identificador de copia ou geracao valido: so hex minusculo com o tamanho certo.
pub fn is_hex_of_len(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Denuncia guardada no relay (bytes opacos, cifrados para o operador).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct StoredReport {
    pub id: u64,
    /// Hora em que o relay a recebeu (ms desde 1970).
    pub received_at_ms: u64,
    pub sealed: Vec<u8>,
}

/// Motivo do `Error` a uma conta banida (na autenticacao e no registo).
pub const BANNED: &str = "conta banida";
/// Motivo do `Error` quando uma conta passa o limite diario de denuncias.
pub const REPORT_LIMIT: &str = "limite de denuncias atingido, tenta amanha";
/// Motivo do `Error` a um pedido de operador vindo de outra conta.
pub const NOT_OPERATOR: &str = "so o operador";

/// One-time prekeys guardadas por conta, no maximo.
pub const MAX_OTPKS: usize = 200;
/// Tamanho de uma one-time prekey guardada: `[4 numero BE][32 publica]`.
pub const OTPK_LEN: usize = 36;
/// Motivo do `Error` a um registo de conta nova sem prova de trabalho valida.
pub const POW_REQUIRED: &str = "prova de trabalho em falta ou invalida";
/// Motivo do `Error` quando uma conta passa o limite de pedidos.
/// Acao paga (anexo ou chamada) de uma conta sem premium, com o bloqueio ligado (0.9.21).
pub const PREMIUM_REQUIRED: &str = "precisa de premium";

pub const RATE_LIMITED: &str = "demasiados pedidos, tenta daqui a pouco";

/// Motivo do `Error` a `IssueFichas` num relay sem a semente das fichas.
pub const FICHAS_OFF: &str = "fichas desligadas neste relay";

/// Tamanho maximo de um token FCM (na pratica ~160 a 200 caracteres).
pub const MAX_PUSH_TOKEN: usize = 4096;

/// Token FCM aceite: so letras, digitos e `-_:.` (o formato que a Google usa).
pub fn is_valid_push_token(t: &str) -> bool {
    !t.is_empty() && t.len() <= MAX_PUSH_TOKEN && t.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b))
}

/// Motivo do `Error` a um `SendMessage` para quem apagou a conta.
pub const RECIPIENT_DELETED: &str = "o destinatario apagou a conta";

/// Motivo do `Error` a um registo de conta nova que outro relay da federacao ja tem
/// (`federation.rs`). A conta continua a existir, so que noutro relay.
pub const ELSEWHERE: &str = "conta noutro relay";

/// `ELSEWHERE` com o nome do relay da conta (fase 3): "conta noutro relay: espanha". A app tem a
/// lista dos relays e muda para esse. So chega a quem prova ser a conta (registo assinado).
pub fn elsewhere(relay: Option<&str>) -> String {
    match relay {
        Some(n) => format!("{ELSEWHERE}: {n}"),
        None => ELSEWHERE.to_string(),
    }
}

/// Motivo do `Error` a um registo de conta nova num relay que ja nao aceita contas novas
/// (`RELAY_NOVAS_CONTAS=0`, decidido pelo operador). A app tenta o relay seguinte da lista.
pub const NO_NEW_ACCOUNTS: &str = "relay sem contas novas";

/// Motivo do `Error` quando o destinatario esta noutro relay da federacao e esse relay nao
/// respondeu. A mensagem fica na outbox de quem envia e segue no ciclo seguinte.
pub const FED_UNAVAILABLE: &str = "relay do destinatario sem resposta, tenta daqui a pouco";

/// Motivo do `Error` a quem envia (mensagem, token de notificacoes, anexo) sem ter
/// bundle no relay. As apps registam o bundle sempre que abrem; o que ficar por
/// enviar continua na outbox e segue no ciclo seguinte.
pub const NOT_REGISTERED: &str = "conta sem registo no relay";

/// Motivo do `Error` a um `SendMessage` para um ID sem bundle (nunca existiu ou nao
/// abre a app ha mais de 90 dias). A mensagem fica na outbox de quem envia.
pub const RECIPIENT_UNKNOWN: &str = "destinatario desconhecido";

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
    /// `one_time_pre_key`: uma one-time prekey do dono do bundle (`[4 numero][32
    /// publica]`), ja apagada do relay. Ausente se ja nao houver (clientes antigos
    /// ignoram o campo).
    PreKeyBundle {
        bundle_bytes: Vec<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        one_time_pre_key: Option<Vec<u8>>,
    },
    PreKeyBundleNotFound,
    Messages { messages: Vec<DeliveredMessage> },
    Error { reason: String },
    BlobCreated { blob_id: String, token: String },
    BlobStatus { size: u64, received: u64, complete: bool },
    /// `data` em base64.
    BlobData { offset: u64, size: u64, data: String },
    BlobNotFound,
    /// Resposta a `AdminListReports`.
    Reports { reports: Vec<StoredReport>, more: bool },
    /// Resposta a `AdminListBans`.
    Bans { user_ids: Vec<UserId> },
    /// Resposta a `BackupGet`: `data` em base64.
    BackupPart { part: u32, parts: u32, data: String },
    BackupNotFound,
    /// Resposta a `PremiumStatus`: ate quando (ms desde 1970); 0 = sem premium. `enforced`
    /// (0.9.21): o bloqueio premium esta ligado (relays antigos: ausente = `false`).
    Premium {
        until_ms: u64,
        #[serde(default)]
        enforced: bool,
        /// (0.9.23) O premium esta ativo agora, pelo relogio do relay: as apps deixam de
        /// depender do relogio do telemovel. Relays antigos: ausente.
        #[serde(default)]
        active: bool,
    },
    /// Resposta a `AdminPremiumEnforce`: estado do bloqueio depois do pedido.
    PremiumEnforce { on: bool },
    /// Resposta a `AdminListPremium`.
    PremiumList { entries: Vec<PremiumEntry> },
    /// Resposta a `IssueFichas`: as primeiras `granted` fichas pedidas, avaliadas
    /// (`evaluated`: base64 de granted x 32 bytes) com a prova DLEQ em lote (`proof`: base64
    /// de 64 bytes) para a chave publica do mes (`public_key`: base64 de 32 bytes). `epoch`
    /// diferente do pedido: o relay ja esta noutro mes (pedir outra vez para esse, sem nada
    /// dado). `remaining`: quantas esta conta ainda pode pedir neste mes; `per_month`: o teto.
    Fichas { epoch: u32, public_key: String, evaluated: String, proof: String, granted: u32, remaining: u32, per_month: u32 },
    /// Resposta a `AdminOnline`.
    Online { users: u64 },
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
