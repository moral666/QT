//! API de ALTO NIVEL para a app Android: um unico objeto `QtClient` que
//! guarda todas as chaves e estados DENTRO do Rust (SQLCipher). Kotlin so
//! manipula bytes publicos, strings e IDs de contacto - nunca chaves
//! privadas, root/chain/message keys, estados de ratchet nem KDFs.
//!
//! Fluxo: PQXDH (X25519 + ML-KEM-768) -> Double Ratchet -> (opcional)
//! Sealed Sender. Ver docs/protocol-spec.md ("Fluxo de sessao da app").
//!
//! Formato do ENVELOPE (v1) - e o unico que o relay ve (opaco para ele):
//!   [1 versao=1][1 tipo: 0=mensagem, 1=mensagem com cabecalho PQXDH]
//!   se tipo=1: [32 ephemeral_pub][4 opk_id BE (0=nenhuma)][4 pq_prekey_id BE][1088 ct ML-KEM]
//!   [32 dh_public do ratchet][4 pn BE][4 n BE][ciphertext AEAD]
//! Envelope SELADO: [1 versao=1][2 len BE][caixa selada com identidade do
//! remetente][envelope]. O cabecalho PQXDH NAO contem a identidade do
//! remetente (vai dentro da caixa selada, ou e dada pelo relay no modo normal).

use std::sync::{Arc, Mutex};

use qt_core::pqxdh::{
    pq_ciphertext_from_bytes, pq_public_key_from_bytes, pqxdh_initiate, pqxdh_respond, PqPreKeyBundle,
    PqPreKeyPair, PQ_CIPHERTEXT_LEN, PQ_PUBLIC_KEY_LEN,
};
use qt_core::identity::{self, session_associated_data};
use qt_core::primitives::{DhKeyPair, SigningKeyPair};
use qt_core::ratchet::{EncryptedMessage, RatchetState};
use qt_core::sealed_sender::{seal_sender_identity_bound, unseal_sender_identity_bound};
use sha2::{Digest, Sha256};
use qt_storage as st;
use x25519_dalek::PublicKey;
use zeroize::Zeroize;

const ENV_VERSION: u8 = 2; // v2: cabecalho do ratchet com PN; AD de sessao; caixa selada ligada ao envelope
const KIND_MSG: u8 = 0;
const KIND_PREKEY_MSG: u8 = 1;
const HEADER_LEN: usize = 32 + 4 + 4 + PQ_CIPHERTEXT_LEN;
const PQ_PREKEY_ID: u32 = 1;
const MAX_ENVELOPE_LEN: usize = 1024 * 1024;
// Limite de plaintext por mensagem. Os frames do relay sao JSON dentro de uma
// mensagem Noise (max 65535 bytes) e cada byte vira ate 4 caracteres JSON:
// 10 000 bytes de texto + overhead do envelope cabem com folga. Anexos
// grandes ficam FORA do ambito desta fase (exigem fragmentacao).
pub const MAX_PLAINTEXT_LEN: usize = 10_000;
const AUTH_CONTEXT: &[u8] = b"QT-RELAY-AUTH-v1";
const MAX_OTPK_BATCH: u32 = 500;
/// Chave efemera PQXDH do contacto que criou a sessao atual (lado que respondeu).
pub(crate) const KV_PEER_EPH: &str = "peer_eph:";

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum QtError {
    #[error("entrada invalida: {field}")]
    InvalidInput { field: String },
    #[error("erro de armazenamento seguro")]
    Storage,
    #[error("nao existe identidade")]
    IdentityMissing,
    #[error("ja existe uma identidade")]
    IdentityExists,
    #[error("sessao inexistente")]
    UnknownSession,
    #[error("envelope duplicado (replay)")]
    Replay,
    #[error("envelope invalido ou corrompido")]
    InvalidEnvelope,
    #[error("bundle de chaves invalido")]
    InvalidBundle,
    #[error("prekey inexistente ou ja utilizada")]
    PrekeyUnavailable,
    #[error("falha criptografica")]
    Crypto,
}

impl From<st::StorageError> for QtError {
    fn from(e: st::StorageError) -> Self {
        match e {
            st::StorageError::IdentityNotFound => QtError::IdentityMissing,
            st::StorageError::PrekeyNotFoundOrUsed => QtError::PrekeyUnavailable,
            // Mensagem generica: nunca propagar detalhes que possam conter material sensivel.
            _ => QtError::Storage,
        }
    }
}
impl From<st::rusqlite::Error> for QtError {
    fn from(_: st::rusqlite::Error) -> Self {
        QtError::Storage
    }
}

pub(crate) fn bad(field: &str) -> QtError {
    QtError::InvalidInput { field: field.to_string() }
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}
pub(crate) fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() & 1 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect()
}
fn arr32(b: &[u8], field: &str) -> Result<[u8; 32], QtError> {
    b.try_into().map_err(|_| bad(field))
}

// ---------------------------------------------------------------- records

#[derive(uniffi::Record)]
pub struct PublicIdentity {
    pub identity_public: Vec<u8>,
    pub signing_public: Vec<u8>,
    /// Identificador da caixa de correio no relay: hex(signing_public).
    pub relay_id: String,
    /// "QT Public ID": "qt1:" + hex(identity_public || signing_public).
    pub public_id: String,
}

#[derive(uniffi::Record, Clone)]
pub struct OneTimePreKeyPublic {
    pub id: u32,
    pub public: Vec<u8>,
}

/// Bundle a PUBLICAR no relay (varias one-time prekeys).
#[derive(uniffi::Record)]
pub struct PublishedBundle {
    pub identity_public: Vec<u8>,
    pub signing_public: Vec<u8>,
    pub signed_pre_key_public: Vec<u8>,
    pub signed_pre_key_signature: Vec<u8>,
    pub pq_pre_key_id: u32,
    pub pq_pre_key_public: Vec<u8>,
    pub one_time_pre_keys: Vec<OneTimePreKeyPublic>,
}

/// Bundle de um CONTACTO, tal como o relay o entrega (no maximo 1 one-time prekey).
#[derive(uniffi::Record)]
pub struct PeerBundle {
    pub identity_public: Vec<u8>,
    pub signing_public: Vec<u8>,
    pub signed_pre_key_public: Vec<u8>,
    pub signed_pre_key_signature: Vec<u8>,
    pub pq_pre_key_id: u32,
    pub pq_pre_key_public: Vec<u8>,
    pub one_time_pre_key: Option<OneTimePreKeyPublic>,
}

#[derive(uniffi::Record)]
pub struct SealedDecrypted {
    pub contact_id: String,
    pub plaintext: Vec<u8>,
}

// ----------------------------------------------------------------- objeto

#[derive(uniffi::Object)]
pub struct QtClient {
    conn: Mutex<st::rusqlite::Connection>,
    /// Pasta dos anexos (ficheiros cifrados), ao lado da base de dados.
    pub(crate) files_dir: std::path::PathBuf,
}

pub(crate) fn parse_public_id(id: &str) -> Result<([u8; 32], [u8; 32]), QtError> {
    let rest = id.strip_prefix("qt1:").ok_or_else(|| bad("public_id"))?;
    let bytes = unhex(rest).ok_or_else(|| bad("public_id"))?;
    if bytes.len() != 64 {
        return Err(bad("public_id"));
    }
    Ok((arr32(&bytes[..32], "public_id")?, arr32(&bytes[32..], "public_id")?))
}

pub(crate) fn contact_to_identity(contact_id: &str) -> Result<PublicKey, QtError> {
    let b = unhex(contact_id).ok_or_else(|| bad("contact_id"))?;
    Ok(PublicKey::from(arr32(&b, "contact_id")?))
}

fn parse_ratchet_part(b: &[u8]) -> Result<EncryptedMessage, QtError> {
    if b.len() < 40 {
        return Err(QtError::InvalidEnvelope);
    }
    let dh: [u8; 32] = b[..32].try_into().map_err(|_| QtError::InvalidEnvelope)?;
    let pn = u32::from_be_bytes(b[32..36].try_into().map_err(|_| QtError::InvalidEnvelope)?);
    let n = u32::from_be_bytes(b[36..40].try_into().map_err(|_| QtError::InvalidEnvelope)?);
    Ok(EncryptedMessage { dh_public: PublicKey::from(dh), pn, n, ciphertext: b[40..].to_vec() })
}

fn ratchet_to_bytes(m: &EncryptedMessage) -> Vec<u8> {
    let mut out = Vec::with_capacity(40 + m.ciphertext.len());
    out.extend_from_slice(m.dh_public.as_bytes());
    out.extend_from_slice(&m.pn.to_be_bytes());
    out.extend_from_slice(&m.n.to_be_bytes());
    out.extend_from_slice(&m.ciphertext);
    out
}

#[uniffi::export]
impl QtClient {
    /// Abre (ou cria) o storage cifrado. `storage_key` sao 32 bytes
    /// protegidos pelo Android Keystore (ver docs/android-integration.md).
    /// O vetor recebido e apagado da memoria Rust apos uso.
    #[uniffi::constructor]
    pub fn open(path: String, storage_key: Vec<u8>) -> Result<Arc<Self>, QtError> {
        let mut key = storage_key;
        let r = st::open_database_with_key(&path, &key);
        key.zeroize();
        let conn = r.map_err(|e| match e {
            st::StorageError::InvalidStorageKey => bad("storage_key"),
            _ => QtError::Storage,
        })?;
        Ok(Arc::new(QtClient { conn: Mutex::new(conn), files_dir: std::path::PathBuf::from(format!("{path}.anexos")) }))
    }

    // ---------------------------------------------------------- identidade

    pub fn has_identity(&self) -> Result<bool, QtError> {
        let c = self.lock()?;
        Ok(st::identity_exists(&c)?)
    }

    /// Cria identidade (X25519 + Ed25519), signed pre-key e prekey PQ.
    /// Falha se ja existir (nao sobrescreve identidade por acidente).
    pub fn create_identity(&self) -> Result<PublicIdentity, QtError> {
        let c = self.lock()?;
        if st::identity_exists(&c)? {
            return Err(QtError::IdentityExists);
        }
        let ident = st::StoredIdentity {
            identity: DhKeyPair::generate(),
            identity_signing: SigningKeyPair::generate(),
            signed_pre_key: DhKeyPair::generate(),
            // Coluna legada (compat. com a CLI); a API nova usa `one_time_prekeys`.
            one_time_pre_key: DhKeyPair::generate(),
        };
        let tx = c.unchecked_transaction()?;
        st::save_identity(&tx, &ident)?;
        st::save_pq_prekey(&tx, PQ_PREKEY_ID, &PqPreKeyPair::generate())?;
        tx.commit()?;
        Ok(public_identity(&ident))
    }

    /// Informacao PUBLICA da identidade (para mostrar/partilhar o QT Public ID).
    pub fn export_public_identity(&self) -> Result<PublicIdentity, QtError> {
        let c = self.lock()?;
        Ok(public_identity(&st::load_identity(&c)?))
    }

    /// Apaga identidade, prekeys, sessoes e estado Noise (secure_delete + VACUUM).
    pub fn delete_identity(&self) -> Result<(), QtError> {
        let c = self.lock()?;
        st::wipe_all(&c)?;
        // Os anexos estao cifrados com chaves que estavam na base de dados (ja
        // apagadas); os ficheiros vao tambem.
        let _ = std::fs::remove_dir_all(&self.files_dir);
        Ok(())
    }

    /// Gera `count` one-time prekeys novas (ids nunca reutilizados) e devolve
    /// o bundle publico a publicar no relay. As privadas ficam no storage.
    pub fn generate_prekey_bundle(&self, count: u32) -> Result<PublishedBundle, QtError> {
        if count > MAX_OTPK_BATCH {
            return Err(bad("count"));
        }
        let c = self.lock()?;
        let ident = st::load_identity(&c)?;
        let pq = st::load_pq_prekey(&c, PQ_PREKEY_ID)?.ok_or(QtError::Storage)?;
        let tx = c.unchecked_transaction()?;
        let mut next = match st::kv_get(&tx, "opk_next")? {
            Some(b) => u32::from_be_bytes(b.as_slice().try_into().map_err(|_| QtError::Storage)?),
            None => 1,
        };
        let mut otpks = Vec::new();
        for _ in 0..count {
            let kp = DhKeyPair::generate();
            st::save_one_time_prekey(&tx, next, &kp)?;
            otpks.push(OneTimePreKeyPublic { id: next, public: kp.public.as_bytes().to_vec() });
            next = next.checked_add(1).ok_or(QtError::Storage)?;
        }
        st::kv_set(&tx, "opk_next", &next.to_be_bytes())?;
        tx.commit()?;
        let sig = qt_core::pqxdh::sign_pre_key(&ident.identity_signing, &ident.identity.public, &ident.signed_pre_key.public, &pq.public_key_bytes());
        Ok(PublishedBundle {
            identity_public: ident.identity.public.as_bytes().to_vec(),
            signing_public: ident.identity_signing.verifying_key.to_bytes().to_vec(),
            signed_pre_key_public: ident.signed_pre_key.public.as_bytes().to_vec(),
            signed_pre_key_signature: sig.to_bytes().to_vec(),
            pq_pre_key_id: PQ_PREKEY_ID,
            pq_pre_key_public: pq.public_key_bytes(),
            one_time_pre_keys: otpks,
        })
    }

    // ------------------------------------------------------------ sessoes

    /// Adicionar contacto + iniciar sessao PQXDH. `public_id` e o QT Public ID
    /// que o utilizador recebeu fora de banda; o bundle vem do relay e e
    /// VERIFICADO contra esse ID (chaves de identidade) e pela assinatura da
    /// signed pre-key. Devolve o `contact_id` a usar nas restantes chamadas.
    pub fn start_session(&self, public_id: String, bundle: PeerBundle) -> Result<String, QtError> {
        let (id_pub, sign_pub) = parse_public_id(&public_id)?;
        if bundle.identity_public != id_pub || bundle.signing_public != sign_pub {
            return Err(QtError::InvalidBundle);
        }
        let their_spk = PublicKey::from(arr32(&bundle.signed_pre_key_public, "signed_pre_key_public")?);
        let sig: [u8; 64] = bundle
            .signed_pre_key_signature
            .as_slice()
            .try_into()
            .map_err(|_| QtError::InvalidBundle)?;
        let verifying =
            ed25519_dalek::VerifyingKey::from_bytes(&sign_pub).map_err(|_| QtError::InvalidBundle)?;
        if bundle.pq_pre_key_public.len() != PQ_PUBLIC_KEY_LEN {
            return Err(QtError::InvalidBundle);
        }
        let pq_pub = pq_public_key_from_bytes(&bundle.pq_pre_key_public).map_err(|_| QtError::InvalidBundle)?;
        let (opk_id, opk_pub) = match &bundle.one_time_pre_key {
            Some(o) if o.id != 0 => (o.id, Some(PublicKey::from(arr32(&o.public, "one_time_pre_key")?))),
            Some(_) => return Err(QtError::InvalidBundle),
            None => (0, None),
        };
        let contact_id = hex(&id_pub);

        let c = self.lock()?;
        let ident = st::load_identity(&c)?;
        if st::load_session(&c, &contact_id)?.is_some() {
            return Err(bad("contact_id (sessao ja existe)"));
        }
        let pq_bundle = PqPreKeyBundle {
            identity_key: PublicKey::from(id_pub),
            identity_signing_key: verifying,
            signed_pre_key: their_spk,
            signed_pre_key_signature: ed25519_dalek::Signature::from_bytes(&sig),
            pq_pre_key: pq_pub,
            one_time_pre_key: opk_pub,
        };
        let mut init = pqxdh_initiate(&ident.identity, &pq_bundle).map_err(|_| QtError::InvalidBundle)?;
        let ratchet = RatchetState::init_as_initiator(init.shared_secret, their_spk).with_associated_data(
            session_associated_data(&ident.identity.public, &PublicKey::from(id_pub)),
        );
        init.shared_secret.zeroize();

        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(init.ephemeral_public.as_bytes());
        header.extend_from_slice(&opk_id.to_be_bytes());
        header.extend_from_slice(&bundle.pq_pre_key_id.to_be_bytes());
        header.extend_from_slice(init.pq_ciphertext.as_slice());

        let tx = c.unchecked_transaction()?;
        st::save_session(&tx, &contact_id, &ratchet)?;
        st::save_session_meta(
            &tx,
            &st::SessionMeta {
                contact_id: contact_id.clone(),
                peer_identity_public: id_pub.to_vec(),
                protocol: "pqxdh".into(),
                last_recv_seq: 0,
                send_seq: 0,
            },
        )?;
        st::kv_set(&tx, &format!("pending:{contact_id}"), &header)?;
        // Quem inicia a conversa manda o seu Public ID (cifrado) antes da 1.a mensagem:
        // assim quem recebe pode aceitar o convite sem ter de escrever o ID.
        st::kv_set(&tx, &format!("{}{contact_id}", crate::history::KV_SEND_INTRO), &[1])?;
        tx.commit()?;
        Ok(contact_id)
    }

    pub fn has_session(&self, contact_id: String) -> Result<bool, QtError> {
        let c = self.lock()?;
        Ok(st::load_session(&c, &contact_id)?.is_some())
    }

    pub fn delete_session(&self, contact_id: String) -> Result<(), QtError> {
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        st::delete_session(&tx, &contact_id)?;
        st::kv_delete(&tx, &format!("pending:{contact_id}"))?;
        st::kv_delete(&tx, &format!("{KV_PEER_EPH}{contact_id}"))?;
        tx.commit()?;
        Ok(())
    }

    /// Cifra para um contacto e AVANCA o ratchet (estado persistido
    /// atomicamente). Devolve o envelope opaco a enviar ao relay.
    pub fn encrypt(&self, contact_id: String, plaintext: Vec<u8>) -> Result<Vec<u8>, QtError> {
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        let env = self.encrypt_with(&tx, &contact_id, &plaintext)?;
        tx.commit()?;
        Ok(env)
    }

    /// Decifra um envelope vindo de `contact_id` (no modo normal, o relay
    /// indica o remetente). Rejeita duplicados/replay e corrupcao; um
    /// envelope invalido NAO altera o estado persistido.
    pub fn decrypt(&self, contact_id: String, envelope: Vec<u8>) -> Result<Vec<u8>, QtError> {
        if envelope.len() > MAX_ENVELOPE_LEN {
            return Err(QtError::InvalidEnvelope);
        }
        let _ = contact_to_identity(&contact_id)?;
        let c = self.lock()?;
        self.decrypt_locked(&c, &contact_id, &envelope)
    }

    /// Bundle PUBLICO para registar no relay (formato binario v3, 1349 bytes):
    /// [1 ver=3][32 identity][32 signing][32 signed_prekey][64 assinatura][4 pq_id BE][1184 pq_pub].
    /// A assinatura Ed25519 cobre a identidade X25519 (liga as duas chaves).
    /// Sem one-time prekeys: o relay atual nao as distribui de forma
    /// single-use (ver docs); PQXDH sem OTPK continua valido.
    pub fn publishable_bundle(&self) -> Result<Vec<u8>, QtError> {
        let c = self.lock()?;
        let ident = st::load_identity(&c)?;
        let pq = st::load_pq_prekey(&c, PQ_PREKEY_ID)?.ok_or(QtError::Storage)?;
        identity::encode_bundle(
            &ident.identity.public,
            &ident.identity_signing,
            &ident.signed_pre_key.public,
            PQ_PREKEY_ID,
            &pq.public_key_bytes(),
        )
        .ok_or(QtError::Crypto)
    }

    /// SAFETY NUMBER (60 digitos) entre ti e o dono de `peer_public_id`, para
    /// comparar por outro canal (voz, pessoalmente). Se ja existir sessao com
    /// esse contacto, a identidade do ID tem de ser a da sessao.
    pub fn safety_number(&self, peer_public_id: String) -> Result<String, QtError> {
        let (peer_id, peer_sign) = parse_public_id(&peer_public_id)?;
        let c = self.lock()?;
        let ident = st::load_identity(&c)?;
        if let Some(meta) = st::load_session_meta(&c, &hex(&peer_id))? {
            if meta.peer_identity_public != peer_id {
                return Err(QtError::InvalidBundle);
            }
        }
        Ok(identity::safety_number(
            &ident.identity.public.to_bytes(),
            &ident.identity_signing.verifying_key.to_bytes(),
            &peer_id,
            &peer_sign,
        ))
    }

    // ------------------------------------------------------------- relay

    /// Nova sessao Noise (iniciador) com a chave estatica Noise deste
    /// dispositivo, guardada no storage cifrado (gerada na 1.a utilizacao).
    /// A app so move bytes entre esta sessao e o WebSocket.
    pub fn relay_noise_session(&self) -> Result<Arc<crate::NoiseSession>, QtError> {
        let c = self.lock()?;
        let mut key = match st::load_noise_static(&c)? {
            Some(k) => k,
            None => {
                let kp = qt_transport::generate_static_keypair().map_err(|_| QtError::Crypto)?;
                st::save_noise_static(&c, &kp.private)?;
                kp.private
            }
        };
        let r = crate::NoiseSession::initiator_from_key(&key);
        key.zeroize();
        r.map_err(|_| QtError::Crypto)
    }

    /// Caixa de correio deste utilizador no relay.
    pub fn relay_user_id(&self) -> Result<String, QtError> {
        let c = self.lock()?;
        Ok(public_identity(&st::load_identity(&c)?).relay_id)
    }

    /// Primeira mensagem (a cifrar pelo Noise) de cada ligacao ao relay:
    /// assinatura Ed25519 sobre `contexto || hash do handshake`.
    pub fn relay_auth_message(&self, handshake_hash: Vec<u8>) -> Result<Vec<u8>, QtError> {
        if handshake_hash.len() != 32 {
            return Err(bad("handshake_hash"));
        }
        let c = self.lock()?;
        let ident = st::load_identity(&c)?;
        let mut m = AUTH_CONTEXT.to_vec();
        m.extend_from_slice(&handshake_hash);
        let sig = ident.identity_signing.sign(&m);
        json_bytes(serde_json::json!({"Authenticate": {
            "signing_public": ident.identity_signing.verifying_key.to_bytes().to_vec(),
            "signature": sig.to_bytes().to_vec(),
        }}))
    }

    pub fn relay_request_register_bundle(&self, bundle_bytes: Vec<u8>) -> Result<Vec<u8>, QtError> {
        let uid = self.relay_user_id()?;
        json_bytes(serde_json::json!({"RegisterPreKeyBundle": {"user_id": uid, "bundle_bytes": bundle_bytes}}))
    }

    pub fn relay_request_fetch_bundle(&self, relay_id: String) -> Result<Vec<u8>, QtError> {
        json_bytes(serde_json::json!({"FetchPreKeyBundle": {"user_id": relay_id}}))
    }

    /// `blob` e o resultado de `encrypt`/`encrypt_sealed`. `sealed_from`
    /// fica vazio: o remetente (quando selado) ja vai dentro do blob.
    pub fn relay_request_send(&self, to_relay_id: String, blob: Vec<u8>) -> Result<Vec<u8>, QtError> {
        json_bytes(serde_json::json!({"SendMessage": {"to": to_relay_id, "sealed_from": Vec::<u8>::new(), "ciphertext": blob}}))
    }

    pub fn relay_request_fetch(&self) -> Result<Vec<u8>, QtError> {
        let uid = self.relay_user_id()?;
        json_bytes(serde_json::json!({"FetchMessages": {"user_id": uid}}))
    }

    /// Como `relay_request_fetch`, mas o relay so responde quando houver mensagens
    /// (ou ao fim de `timeout_secs`, no maximo 25): entrega imediata. Usar numa
    /// ligacao propria (fica ocupada durante a espera), com tempo de resposta maior
    /// do que `timeout_secs`. Resposta `Messages` (vazia se o tempo acabou), a tratar
    /// como a de `relay_request_fetch`. Um relay antigo responde `Error`: voltar a
    /// `relay_request_fetch` periodico.
    pub fn relay_request_wait(&self, timeout_secs: u32) -> Result<Vec<u8>, QtError> {
        let uid = self.relay_user_id()?;
        json_bytes(serde_json::json!({"WaitMessages": {"user_id": uid, "timeout_secs": timeout_secs}}))
    }

    /// Apaga a conta deste utilizador no relay: bundle de pre-keys, fila de mensagens
    /// e contagem de anexos. Resposta `Ack`. Depois, quem tentar enviar-lhe uma mensagem
    /// recebe `Error` com `relay_recipient_deleted_reason()`. Um relay antigo responde
    /// `Error` (pedido desconhecido). Chamar ANTES de `delete_identity` (depois ja nao ha
    /// chave para autenticar).
    pub fn relay_request_delete_account(&self) -> Result<Vec<u8>, QtError> {
        let uid = self.relay_user_id()?;
        json_bytes(serde_json::json!({"DeleteAccount": {"user_id": uid}}))
    }

    /// Confirmar so DEPOIS de `decrypt` ter persistido cada mensagem.
    pub fn relay_request_ack(&self, ids: Vec<u64>) -> Result<Vec<u8>, QtError> {
        let uid = self.relay_user_id()?;
        json_bytes(serde_json::json!({"AckMessages": {"user_id": uid, "ids": ids}}))
    }

    // ------------------------------------------------------- sealed sender

    /// Como `encrypt`, mas esconde o remetente do relay: a identidade do
    /// remetente segue numa caixa selada para o destinatario.
    pub fn encrypt_sealed(&self, contact_id: String, plaintext: Vec<u8>) -> Result<Vec<u8>, QtError> {
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        let out = self.seal_with(&tx, &contact_id, &plaintext)?;
        tx.commit()?;
        Ok(out)
    }

    /// Abre a caixa selada, descobre o remetente e decifra.
    pub fn decrypt_sealed(&self, sealed_envelope: Vec<u8>) -> Result<SealedDecrypted, QtError> {
        let c = self.lock()?;
        let (contact_id, plaintext) = self.open_sealed_with(&c, &sealed_envelope, &mut |_, _, _, _| Ok(()))?;
        Ok(SealedDecrypted { contact_id, plaintext })
    }
}

impl QtClient {
    pub(crate) fn lock(&self) -> Result<std::sync::MutexGuard<'_, st::rusqlite::Connection>, QtError> {
        self.conn.lock().map_err(|_| QtError::Storage)
    }

    /// Cifra e avanca o ratchet usando `c` (o CHAMADOR abre e confirma a
    /// transacao: assim pode guardar mais coisas na mesma transacao).
    pub(crate) fn encrypt_with(&self, c: &st::rusqlite::Connection, contact_id: &str, plaintext: &[u8]) -> Result<Vec<u8>, QtError> {
        if plaintext.len() > MAX_PLAINTEXT_LEN {
            return Err(bad("plaintext"));
        }
        let mut state = st::load_session(c, contact_id)?.ok_or(QtError::UnknownSession)?;
        let msg = state.encrypt(plaintext).map_err(|_| QtError::Crypto)?;
        let pending = st::kv_get(c, &format!("pending:{contact_id}"))?;
        let mut env = vec![ENV_VERSION, if pending.is_some() { KIND_PREKEY_MSG } else { KIND_MSG }];
        if let Some(h) = pending {
            env.extend_from_slice(&h);
        }
        env.extend_from_slice(&ratchet_to_bytes(&msg));
        st::save_session(c, contact_id, &state)?;
        if let Some(mut m) = st::load_session_meta(c, contact_id)? {
            m.send_seq += 1;
            st::save_session_meta(c, &m)?;
        }
        Ok(env)
    }

    /// `encrypt_with` + caixa selada com a identidade do remetente.
    pub(crate) fn seal_with(&self, c: &st::rusqlite::Connection, contact_id: &str, plaintext: &[u8]) -> Result<Vec<u8>, QtError> {
        let recipient = contact_to_identity(contact_id)?;
        // Ponto unico por onde passa tudo o que sai (mensagens, imagens, recibos).
        if crate::history::is_blocked_in(c, contact_id)? {
            return Err(crate::history::blocked_error());
        }
        let env = self.encrypt_with(c, contact_id, plaintext)?;
        let me = st::load_identity(c)?.identity.public;
        let sealed = seal_sender_identity_bound(&me, &recipient, &envelope_binding(&env)).map_err(|_| QtError::Crypto)?;
        let mut out = vec![ENV_VERSION];
        out.extend_from_slice(&(sealed.len() as u16).to_be_bytes());
        out.extend_from_slice(&sealed);
        out.extend_from_slice(&env);
        Ok(out)
    }

    /// Abre um envelope selado e decifra-o. `after` corre DENTRO da transacao
    /// que avanca o ratchet (recebe a ligacao, o contacto, o id do envelope e
    /// o plaintext): se `after` falhar, nada e persistido e o relay volta a
    /// entregar a mensagem mais tarde.
    pub(crate) fn open_sealed_with(
        &self,
        c: &st::rusqlite::Connection,
        sealed_envelope: &[u8],
        after: &mut AfterDecrypt<'_>,
    ) -> Result<(String, Vec<u8>), QtError> {
        if sealed_envelope.len() < 3 || sealed_envelope.len() > MAX_ENVELOPE_LEN || sealed_envelope[0] != ENV_VERSION {
            return Err(QtError::InvalidEnvelope);
        }
        let len = u16::from_be_bytes([sealed_envelope[1], sealed_envelope[2]]) as usize;
        if sealed_envelope.len() < 3 + len {
            return Err(QtError::InvalidEnvelope);
        }
        let ident = st::load_identity(c)?;
        let inner = &sealed_envelope[3 + len..];
        // A identidade selada e so uma PISTA para escolher a sessao. O remetente
        // so e dado como autenticado se o AEAD do ratchet (chaves + AD com as
        // duas identidades) passar com essa sessao.
        let hint = unseal_sender_identity_bound(&ident.identity, &sealed_envelope[3..3 + len], &envelope_binding(inner))
            .map_err(|_| QtError::InvalidEnvelope)?;
        let contact_id = hex(hint.as_bytes());
        let plaintext = self.decrypt_locked_with(c, &contact_id, inner, after)?;
        Ok((contact_id, plaintext))
    }

    fn decrypt_locked(
        &self,
        c: &st::rusqlite::Connection,
        contact_id: &str,
        env: &[u8],
    ) -> Result<Vec<u8>, QtError> {
        self.decrypt_locked_with(c, contact_id, env, &mut |_, _, _, _| Ok(()))
    }

    fn decrypt_locked_with(
        &self,
        c: &st::rusqlite::Connection,
        contact_id: &str,
        env: &[u8],
        after: &mut AfterDecrypt<'_>,
    ) -> Result<Vec<u8>, QtError> {
        if env.len() < 2 || env[0] != ENV_VERSION {
            return Err(QtError::InvalidEnvelope);
        }
        let kind = env[1];
        let (header, ratchet_bytes) = match kind {
            KIND_MSG => (None, &env[2..]),
            KIND_PREKEY_MSG => {
                if env.len() < 2 + HEADER_LEN {
                    return Err(QtError::InvalidEnvelope);
                }
                (Some(&env[2..2 + HEADER_LEN]), &env[2 + HEADER_LEN..])
            }
            _ => return Err(QtError::InvalidEnvelope),
        };
        let msg = parse_ratchet_part(ratchet_bytes)?;
        // ID do envelope = (chave DH do ratchet, n): identifica univocamente a mensagem.
        let mut env_id = msg.dh_public.as_bytes().to_vec();
        env_id.extend_from_slice(&msg.n.to_be_bytes());
        if st::is_envelope_seen(c, contact_id, &env_id)? {
            return Err(QtError::Replay);
        }

        // `state` e sempre uma copia carregada do disco: se a decifragem falhar,
        // nada e persistido.
        let mut used_opk: Option<u32> = None;
        let mut new_session = false;
        // A sessao antiga foi substituida (sessoes cruzadas ou o contacto
        // reiniciou a dele): as nossas mensagens ainda nao entregues tem de ser
        // cifradas de novo com a sessao nova.
        let mut replaced = false;
        let (state, plaintext) = match st::load_session(c, contact_id)? {
            None => {
                let h = header.ok_or(QtError::UnknownSession)?;
                let (mut s, opk) = self.establish_responder(c, contact_id, h)?;
                let p = s.decrypt(&msg).map_err(|_| QtError::Crypto)?;
                used_opk = opk;
                new_session = true;
                (s, p)
            }
            Some(mut s) => match s.decrypt(&msg) {
                Ok(p) => (s, p),
                Err(_) => {
                    // So uma mensagem com cabecalho PQXDH pode abrir uma sessao nova.
                    // Forjar uma exige a chave privada de identidade do contacto
                    // (o PQXDH usa-a e o AEAD autentica as duas identidades), logo
                    // nem o relay nem terceiros conseguem provocar isto.
                    let h = header.ok_or(QtError::Crypto)?;
                    // O mesmo cabecalho que criou a sessao atual NAO e uma sessao nova
                    // (seria voltar atras no tempo): e so uma mensagem que ja nao se
                    // consegue decifrar.
                    if st::kv_get(c, &format!("{KV_PEER_EPH}{contact_id}"))?.as_deref() == Some(&h[..32]) {
                        return Err(QtError::Crypto);
                    }
                    let (mut ns, opk) = self.establish_responder(c, contact_id, h).map_err(|_| QtError::Crypto)?;
                    let p = ns.decrypt(&msg).map_err(|_| QtError::Crypto)?;
                    let ours_pending = st::kv_get(c, &format!("pending:{contact_id}"))?.is_some();
                    if ours_pending {
                        // SESSOES CRUZADAS: os dois iniciaram uma sessao ao mesmo tempo.
                        // Desempate deterministico e igual nos dois lados: fica a sessao
                        // iniciada por quem tem a chave de identidade "menor". O outro
                        // lado adota-a ao receber a nossa mensagem e reenvia as dele.
                        let mine = st::load_identity(c)?.identity.public.to_bytes();
                        let theirs = contact_to_identity(contact_id)?.to_bytes();
                        if theirs >= mine {
                            return Err(QtError::Crypto);
                        }
                    }
                    used_opk = opk;
                    new_session = true;
                    replaced = true;
                    (ns, p)
                }
            },
        };

        let tx = c.unchecked_transaction()?;
        st::save_session(&tx, contact_id, &state)?;
        st::record_envelope(&tx, contact_id, &env_id)?;
        if let Some(id) = used_opk {
            st::delete_one_time_prekey(&tx, id)?;
        }
        st::kv_delete(&tx, &format!("pending:{contact_id}"))?;
        if new_session {
            st::save_session_meta(
                &tx,
                &st::SessionMeta {
                    contact_id: contact_id.to_string(),
                    peer_identity_public: contact_to_identity(contact_id)?.as_bytes().to_vec(),
                    protocol: "pqxdh".into(),
                    last_recv_seq: 0,
                    send_seq: 0,
                },
            )?;
        }
        if let Some(mut m) = st::load_session_meta(&tx, contact_id)? {
            m.last_recv_seq += 1;
            st::save_session_meta(&tx, &m)?;
        }
        if let (true, Some(h)) = (new_session, header) {
            st::kv_set(&tx, &format!("{KV_PEER_EPH}{contact_id}"), &h[..32])?;
        }
        if replaced {
            crate::history::requeue_after_session_reset(self, &tx, contact_id)?;
        }
        // Ganchos do chamador (ex.: guardar a mensagem no historico) na MESMA transacao.
        after(&tx, contact_id, &env_id, &plaintext)?;
        tx.commit()?;
        Ok(plaintext)
    }

    fn establish_responder(
        &self,
        c: &st::rusqlite::Connection,
        contact_id: &str,
        header: &[u8],
    ) -> Result<(RatchetState, Option<u32>), QtError> {
        let their_identity = contact_to_identity(contact_id)?;
        let eph = PublicKey::from(arr32(&header[..32], "header")?);
        let opk_id = u32::from_be_bytes(header[32..36].try_into().map_err(|_| QtError::InvalidEnvelope)?);
        let pq_id = u32::from_be_bytes(header[36..40].try_into().map_err(|_| QtError::InvalidEnvelope)?);
        let ct = pq_ciphertext_from_bytes(&header[40..]).map_err(|_| QtError::InvalidEnvelope)?;

        let ident = st::load_identity(c)?;
        let pq = st::load_pq_prekey(c, pq_id)?.ok_or(QtError::PrekeyUnavailable)?;
        // Le a OPK sem a consumir: so e apagada depois de a 1.a mensagem autenticar.
        let opk = if opk_id == 0 { None } else { Some(st::load_one_time_prekey(c, opk_id)?) };
        let mut secret = pqxdh_respond(
            &ident.identity,
            &ident.signed_pre_key,
            &pq,
            opk.as_ref(),
            &their_identity,
            &eph,
            &ct,
        )
        .map_err(|_| QtError::Crypto)?;
        let state = RatchetState::init_as_responder(secret, ident.signed_pre_key)
            .with_associated_data(session_associated_data(&their_identity, &ident.identity.public));
        secret.zeroize();

        Ok((state, if opk_id == 0 { None } else { Some(opk_id) }))
    }
}

/// Gancho chamado dentro da transacao de decifragem:
/// (ligacao, contact_id, id do envelope, plaintext).
pub(crate) type AfterDecrypt<'a> = dyn FnMut(&st::rusqlite::Connection, &str, &[u8], &[u8]) -> Result<(), QtError> + 'a;

fn envelope_binding(env: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(b"QT-SEALED-BIND-v1");
    h.update(env);
    h.finalize().to_vec()
}

pub(crate) fn public_identity(i: &st::StoredIdentity) -> PublicIdentity {
    let idp = i.identity.public.as_bytes().to_vec();
    let sgp = i.identity_signing.verifying_key.to_bytes().to_vec();
    let mut all = idp.clone();
    all.extend_from_slice(&sgp);
    PublicIdentity { relay_id: hex(&sgp), identity_public: idp, signing_public: sgp, public_id: format!("qt1:{}", hex(&all)) }
}

fn json_bytes(v: serde_json::Value) -> Result<Vec<u8>, QtError> {
    serde_json::to_vec(&v).map_err(|_| QtError::InvalidEnvelope)
}

#[derive(uniffi::Record)]
pub struct RelayDelivered {
    pub id: u64,
    /// Para o modo sealed do QtClient vem vazio; usar `decrypt_sealed(ciphertext)`.
    pub sealed_from: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

#[derive(uniffi::Enum)]
pub enum RelayResponse {
    Ack,
    Bundle { bytes: Vec<u8> },
    BundleNotFound,
    Messages { messages: Vec<RelayDelivered> },
    Error { reason: String },
    /// Espaco reservado para um anexo (guardar com `attachment_set_blob`).
    BlobCreated { blob_id: String, token: String },
    /// Quantos bytes de um anexo o relay ja tem.
    BlobStatus { size: u64, received: u64, complete: bool },
    /// Um bloco de um anexo (cifrado; guardar com `attachment_write_chunk`).
    BlobData { offset: u64, size: u64, data: Vec<u8> },
    /// O relay ja nao tem este anexo (expirou ou foi apagado).
    BlobNotFound,
}

#[derive(serde::Deserialize)]
struct WireDelivered {
    id: u64,
    sealed_from: Vec<u8>,
    ciphertext: Vec<u8>,
}
#[derive(serde::Deserialize)]
enum WireServerMsg {
    Ack,
    PreKeyBundle { bundle_bytes: Vec<u8> },
    PreKeyBundleNotFound,
    Messages { messages: Vec<WireDelivered> },
    Error { reason: String },
    BlobCreated { blob_id: String, token: String },
    BlobStatus { size: u64, received: u64, complete: bool },
    BlobData { offset: u64, size: u64, data: String },
    BlobNotFound,
}

/// Interpreta uma resposta do relay (ja decifrada pelo Noise).
#[uniffi::export]
pub fn relay_parse_response(bytes: Vec<u8>) -> Result<RelayResponse, QtError> {
    if bytes.len() > MAX_ENVELOPE_LEN {
        return Err(QtError::InvalidEnvelope);
    }
    let m: WireServerMsg = serde_json::from_slice(&bytes).map_err(|_| QtError::InvalidEnvelope)?;
    Ok(match m {
        WireServerMsg::Ack => RelayResponse::Ack,
        WireServerMsg::PreKeyBundle { bundle_bytes } => RelayResponse::Bundle { bytes: bundle_bytes },
        WireServerMsg::PreKeyBundleNotFound => RelayResponse::BundleNotFound,
        WireServerMsg::Messages { messages } => RelayResponse::Messages {
            messages: messages
                .into_iter()
                .map(|d| RelayDelivered { id: d.id, sealed_from: d.sealed_from, ciphertext: d.ciphertext })
                .collect(),
        },
        WireServerMsg::Error { reason } => RelayResponse::Error { reason },
        WireServerMsg::BlobCreated { blob_id, token } => RelayResponse::BlobCreated { blob_id, token },
        WireServerMsg::BlobStatus { size, received, complete } => RelayResponse::BlobStatus { size, received, complete },
        WireServerMsg::BlobData { offset, size, data } => {
            use base64::Engine;
            let data = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| QtError::InvalidEnvelope)?;
            if data.len() > crate::attachments::BLOB_CHUNK {
                return Err(QtError::InvalidEnvelope);
            }
            RelayResponse::BlobData { offset, size, data }
        }
        WireServerMsg::BlobNotFound => RelayResponse::BlobNotFound,
    })
}

/// Motivo do `Error` do relay a um envio para quem apagou a conta
/// (ver `relay_request_delete_account`).
#[uniffi::export]
pub fn relay_recipient_deleted_reason() -> String {
    "o destinatario apagou a conta".to_string()
}

/// Caixa de correio no relay correspondente a um QT Public ID.
#[uniffi::export]
pub fn relay_id_of_public_id(public_id: String) -> Result<String, QtError> {
    let (_, sign) = parse_public_id(&public_id)?;
    Ok(hex(&sign))
}

const BUNDLE_V1_LEN: usize = identity::BUNDLE_LEN;

/// Interpreta o bundle devolvido pelo relay (formato de `publishable_bundle`).
/// A verificacao criptografica (contra o QT Public ID e a assinatura) e feita
/// por `start_session`.
#[uniffi::export]
pub fn peer_bundle_from_relay_bytes(bytes: Vec<u8>) -> Result<PeerBundle, QtError> {
    if bytes.len() != BUNDLE_V1_LEN || bytes[0] != identity::BUNDLE_VERSION {
        return Err(QtError::InvalidBundle);
    }
    Ok(PeerBundle {
        identity_public: bytes[1..33].to_vec(),
        signing_public: bytes[33..65].to_vec(),
        signed_pre_key_public: bytes[65..97].to_vec(),
        signed_pre_key_signature: bytes[97..161].to_vec(),
        pq_pre_key_id: u32::from_be_bytes(bytes[161..165].try_into().map_err(|_| QtError::InvalidBundle)?),
        pq_pre_key_public: bytes[165..].to_vec(),
        one_time_pre_key: None,
    })
}
