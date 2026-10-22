//! API de ALTO NIVEL para a app Android: um unico objeto `QtClient` que
//! guarda todas as chaves e estados DENTRO do Rust (SQLCipher). Kotlin so
//! manipula bytes publicos, strings e IDs de contacto - nunca chaves
//! privadas, root/chain/message keys, estados de ratchet nem KDFs.
//!
//! Fluxo: PQXDH (X25519 + ML-KEM-768) -> Double Ratchet -> (opcional)
//! Sealed Sender. Ver docs/protocol-spec.md ("Fluxo de sessao da app").
//!
//! Formato do ENVELOPE (v3, desde a app 0.9.3) - e o unico que o relay ve (opaco):
//!   [1 versao=3][1 tipo: 0=mensagem, 2=mensagem com cabecalho PQXDH v2]
//!   se tipo=2: [32 ephemeral_pub][4 opk_id BE (0=nenhuma)][4 pq_prekey_id BE]
//!              [4 spk_id BE][1088 ct ML-KEM]
//!   [32 dh_public do ratchet][4 pn BE][4 n BE][ciphertext AEAD]
//! No v3 o plaintext leva ENCHIMENTO antes de ser cifrado (`pad_plaintext`): o
//! tamanho das mensagens fica num de poucos valores fixos.
//! Ainda se RECEBE o v2 (apps ate a 0.9.2 e sessoes iniciadas antes): tipo 0 ou
//! 1 (cabecalho PQXDH v1 sem spk_id), sem enchimento.
//! Envelope SELADO: [1 versao=2][2 len BE][caixa selada com identidade do
//! remetente][envelope]. O cabecalho PQXDH NAO contem a identidade do
//! remetente (vai dentro da caixa selada, ou e dada pelo relay no modo normal).

use std::sync::{Arc, Mutex};

use qt_core::pqxdh::{
    pq_ciphertext_from_bytes, pqxdh_initiate_v2, pqxdh_respond, pqxdh_respond_v2, PqPreKeyPair, PQ_CIPHERTEXT_LEN,
    PQ_PUBLIC_KEY_LEN,
};
use crate::prekeys;
use qt_core::identity::{self, session_associated_data};
use qt_core::primitives::{DhKeyPair, SigningKeyPair};
use qt_core::ratchet::{EncryptedMessage, RatchetState};
use qt_core::sealed_sender::{seal_sender_identity_bound, unseal_sender_identity_bound};
use sha2::{Digest, Sha256};
use qt_storage as st;
use crate::history::OutboxItem;
use x25519_dalek::PublicKey;
use zeroize::Zeroize;

/// v2 (so a receber): cabecalho do ratchet com PN; AD de sessao; sem enchimento.
const ENV_VERSION_V2: u8 = 2;
/// v3 (atual): como o v2, com enchimento do plaintext e cabecalho PQXDH v2.
const ENV_VERSION: u8 = 3;
/// Envelope selado (caixa com a identidade do remetente): formato igual desde a v2.
const SEALED_VERSION: u8 = 2;
const KIND_MSG: u8 = 0;
/// Cabecalho PQXDH v1 (so a receber, ou sessoes iniciadas antes da 0.9.3).
const KIND_PREKEY_MSG: u8 = 1;
/// Cabecalho PQXDH v2 (com o numero da signed pre-key).
const KIND_PREKEY_MSG_V2: u8 = 2;
const HEADER_LEN: usize = 32 + 4 + 4 + PQ_CIPHERTEXT_LEN;
const HEADER_V2_LEN: usize = 32 + 4 + 4 + 4 + PQ_CIPHERTEXT_LEN;
pub(crate) const PQ_PREKEY_ID: u32 = prekeys::FIRST_ID;
/// Tamanhos fixos do plaintext com enchimento (o ultimo cabe `MAX_PLAINTEXT_LEN` + 1).
const PAD_BUCKETS: [usize; 4] = [256, 1024, 4096, 10_240];
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

/// Tranca um Mutex mesmo que um panic anterior o tenha deixado "envenenado". O UniFFI
/// apanha os panics do Rust (a app nao morre), mas antes disto o Mutex ficava
/// envenenado e todas as chamadas seguintes falhavam (`Storage`) ate reiniciar a app.
/// Recuperar e seguro: na base de dados, a transacao em curso foi desfeita pelo
/// SQLite ao desenrolar o panic; a presenca e as chamadas sao so estado em memoria.
pub(crate) fn relock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| {
        m.clear_poison();
        e.into_inner()
    })
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

/// Enchimento antes de cifrar: o plaintext, um byte 0x80 e zeros ate ao primeiro
/// tamanho de `PAD_BUCKETS` que caiba. Assim quem ve o tamanho dos envelopes (o
/// relay) so distingue 4 tamanhos, e nao um recibo de uma mensagem curta.
pub(crate) fn pad_plaintext(p: &[u8]) -> Vec<u8> {
    let target = PAD_BUCKETS.iter().copied().find(|b| *b > p.len()).unwrap_or(p.len() + 1);
    let mut out = Vec::with_capacity(target);
    out.extend_from_slice(p);
    out.push(0x80);
    out.resize(target, 0);
    out
}

/// Retira o enchimento de `pad_plaintext`. `None` se o formato nao bater certo.
pub(crate) fn unpad_plaintext(p: &[u8]) -> Option<Vec<u8>> {
    let end = p.iter().rposition(|b| *b != 0)?;
    (p[end] == 0x80).then(|| p[..end].to_vec())
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

/// Bundle de um CONTACTO, tal como o relay o entrega (bundle v4 e, se o relay ainda
/// tiver, uma one-time prekey). Ver `peer_bundle_from_relay_bytes`.
#[derive(uniffi::Record)]
pub struct PeerBundle {
    pub identity_public: Vec<u8>,
    pub signing_public: Vec<u8>,
    pub signed_pre_key_id: u32,
    pub signed_pre_key_public: Vec<u8>,
    /// Hora (segundos unix) em que o contacto criou a signed pre-key; assinada.
    pub created_at: u64,
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
    /// Quem esta online (so em memoria, nunca em disco; ver presence.rs).
    pub(crate) presence: Mutex<crate::presence::Presence>,
    /// Chamada de voz em curso e eventos por entregar (so em memoria; ver call.rs).
    pub(crate) calls: Mutex<crate::call::Calls>,
    /// Recuperacao a meio (palavras ja verificadas): chave e identificador da copia, so em
    /// memoria (ver recovery.rs).
    pub(crate) pending_recovery: Mutex<Option<crate::recovery::PendingRecovery>>,
    /// Chave temporaria para falar com o relay antes de haver identidade (buscar a copia de
    /// recuperacao num telemovel novo). So em memoria.
    pub(crate) ephemeral_signing: Mutex<Option<SigningKeyPair>>,
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
        Ok(Arc::new(QtClient {
            conn: Mutex::new(conn),
            files_dir: std::path::PathBuf::from(format!("{path}.anexos")),
            presence: Mutex::new(Default::default()),
            calls: Mutex::new(Default::default()),
            pending_recovery: Mutex::new(None),
            ephemeral_signing: Mutex::new(None),
        }))
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
        *relock(&self.presence) = Default::default();
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
        let pq_id = prekeys::current_pq_id(&c)?;
        let pq = st::load_pq_prekey(&c, pq_id)?.ok_or(QtError::Storage)?;
        let tx = c.unchecked_transaction()?;
        let otpks = prekeys::new_one_time_prekeys(&tx, count)?
            .into_iter()
            .map(|(id, public)| OneTimePreKeyPublic { id, public: public.to_vec() })
            .collect();
        tx.commit()?;
        let sig = qt_core::pqxdh::sign_pre_key(&ident.identity_signing, &ident.identity.public, &ident.signed_pre_key.public, &pq.public_key_bytes());
        Ok(PublishedBundle {
            identity_public: ident.identity.public.as_bytes().to_vec(),
            signing_public: ident.identity_signing.verifying_key.to_bytes().to_vec(),
            signed_pre_key_public: ident.signed_pre_key.public.as_bytes().to_vec(),
            signed_pre_key_signature: sig.to_bytes().to_vec(),
            pq_pre_key_id: pq_id,
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
        if bundle.pq_pre_key_public.len() != PQ_PUBLIC_KEY_LEN {
            return Err(QtError::InvalidBundle);
        }
        // Bundle demasiado antigo (o contacto nao abre a app ha meses, ou um relay a
        // servir chaves velhas) ou com data no futuro: recusado.
        let t = prekeys::now();
        if bundle.created_at.saturating_add(prekeys::MAX_BUNDLE_AGE_SECS) < t
            || bundle.created_at > t.saturating_add(prekeys::MAX_BUNDLE_FUTURE_SECS)
        {
            return Err(QtError::InvalidBundle);
        }
        let v4 = identity::BundleV4 {
            identity: id_pub,
            signing: sign_pub,
            signed_pre_key_id: bundle.signed_pre_key_id,
            signed_pre_key: arr32(&bundle.signed_pre_key_public, "signed_pre_key_public")?,
            created_at: bundle.created_at,
            signature: bundle.signed_pre_key_signature.as_slice().try_into().map_err(|_| QtError::InvalidBundle)?,
            pq_pre_key_id: bundle.pq_pre_key_id,
            pq_pre_key: bundle.pq_pre_key_public.clone(),
        };
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
        // Verifica a assinatura do bundle (chave Ed25519 = a do QT Public ID) e faz o PQXDH v2.
        let mut init = pqxdh_initiate_v2(&ident.identity, &v4, opk_pub.as_ref()).map_err(|_| QtError::InvalidBundle)?;
        let ratchet = RatchetState::init_as_initiator(init.shared_secret, init.signed_pre_key).with_associated_data(
            session_associated_data(&ident.identity.public, &PublicKey::from(id_pub)),
        );
        init.shared_secret.zeroize();

        let mut header = Vec::with_capacity(HEADER_V2_LEN);
        header.extend_from_slice(init.ephemeral_public.as_bytes());
        header.extend_from_slice(&opk_id.to_be_bytes());
        header.extend_from_slice(&init.pq_pre_key_id.to_be_bytes());
        header.extend_from_slice(&init.signed_pre_key_id.to_be_bytes());
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
        // O contacto tinha terminado a sessao antiga: o que estava por entregar segue ja.
        crate::history::after_new_session(self, &tx, &contact_id)?;
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

    /// Bundle PUBLICO para registar no relay (formato binario v4, ver
    /// `qt_core::identity::encode_bundle_v4`). A assinatura Ed25519 cobre a identidade
    /// X25519, o numero e a data da signed pre-key e a prekey ML-KEM. Se a signed
    /// pre-key tiver mais de 7 dias, e trocada aqui (ver `prekeys::rotate_if_due`).
    pub fn publishable_bundle(&self) -> Result<Vec<u8>, QtError> {
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        let mut ident = st::load_identity(&tx)?;
        let (spk_id, created, pq_id) = prekeys::rotate_if_due(&tx, &mut ident)?;
        let pq = st::load_pq_prekey(&tx, pq_id)?.ok_or(QtError::Storage)?;
        let out = identity::encode_bundle_v4(
            &ident.identity.public,
            &ident.identity_signing,
            spk_id,
            &ident.signed_pre_key.public,
            created,
            pq_id,
            &pq.public_key_bytes(),
        )
        .ok_or(QtError::Crypto)?;
        tx.commit()?;
        Ok(out)
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
        let mut m = AUTH_CONTEXT.to_vec();
        m.extend_from_slice(&handshake_hash);
        let c = self.lock()?;
        // Sem identidade (telemovel novo, a recuperar a conta): uma chave temporaria, so em
        // memoria. Chega para pedir a copia; o relay nao guarda nada dela.
        if !st::identity_exists(&c)? {
            let mut eph = relock(&self.ephemeral_signing);
            let k = eph.get_or_insert_with(SigningKeyPair::generate);
            let sig = k.sign(&m);
            return json_bytes(serde_json::json!({"Authenticate": {
                "signing_public": k.verifying_key.to_bytes().to_vec(),
                "signature": sig.to_bytes().to_vec(),
            }}));
        }
        let ident = st::load_identity(&c)?;
        let sig = ident.identity_signing.sign(&m);
        json_bytes(serde_json::json!({"Authenticate": {
            "signing_public": ident.identity_signing.verifying_key.to_bytes().to_vec(),
            "signature": sig.to_bytes().to_vec(),
        }}))
    }

    /// Pedido de registo do bundle (de `publishable_bundle`). Leva tambem:
    /// - a prova de trabalho da conta (calculada na primeira vez, uns segundos;
    ///   chamar fora da thread principal), que o relay exige a contas novas;
    /// - uma vez por dia, um lote novo de one-time prekeys (substitui o do relay).
    pub fn relay_request_register_bundle(&self, bundle_bytes: Vec<u8>) -> Result<Vec<u8>, QtError> {
        let uid = self.relay_user_id()?;
        let pow = self.pow_nonce()?;
        let opks = {
            let c = self.lock()?;
            let tx = c.unchecked_transaction()?;
            let r = prekeys::one_time_prekeys_to_upload(&tx)?;
            tx.commit()?;
            r
        };
        let mut body = serde_json::json!({"user_id": uid, "bundle_bytes": bundle_bytes, "pow": pow});
        if let Some(keys) = opks {
            body["one_time_pre_keys"] = serde_json::json!(keys);
        }
        json_bytes(serde_json::json!({ "RegisterPreKeyBundle": body }))
    }

    pub fn relay_request_fetch_bundle(&self, relay_id: String) -> Result<Vec<u8>, QtError> {
        json_bytes(serde_json::json!({"FetchPreKeyBundle": {"user_id": relay_id}}))
    }

    /// `blob` e o resultado de `encrypt`/`encrypt_sealed`. `sealed_from`
    /// fica vazio: o remetente (quando selado) ja vai dentro do blob.
    pub fn relay_request_send(&self, to_relay_id: String, blob: Vec<u8>) -> Result<Vec<u8>, QtError> {
        json_bytes(serde_json::json!({"SendMessage": {"to": to_relay_id, "sealed_from": Vec::<u8>::new(), "ciphertext": blob}}))
    }

    /// Como `relay_request_send`, para um item da outbox: recibos, reacoes, convites e
    /// presenca vao com `silent` (o relay nao toca a campainha a uma app fechada; so as
    /// mensagens tocam). Um relay antigo ignora o campo e toca sempre, como antes.
    ///
    /// Chamadas de voz: a oferta e o cancelamento levam `wake` ("Call" / "CallEnd"); o relay
    /// manda a campainha marcada, para a app fechada tocar mesmo com o bloqueio ativo. Um
    /// relay antigo ignora o campo e toca como numa mensagem.
    pub fn relay_request_send_item(&self, to_relay_id: String, item: OutboxItem) -> Result<Vec<u8>, QtError> {
        let mut m = serde_json::json!({
            "to": to_relay_id,
            "sealed_from": Vec::<u8>::new(),
            "ciphertext": item.blob,
            "silent": item.silent,
        });
        if let Some(w) = item.wake {
            m["wake"] = serde_json::json!(match w {
                crate::history::CallWake::Call => "Call",
                crate::history::CallWake::CallEnd => "CallEnd",
            });
        }
        json_bytes(serde_json::json!({ "SendMessage": m }))
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

    /// Notificacoes com a app fechada: regista no relay o token do Firebase Cloud
    /// Messaging deste telemovel (o relay so o usa para pedir a Google que acorde o
    /// telemovel, com uma mensagem vazia). `token` vazio = deixar de receber. Resposta
    /// `Ack`; um relay antigo responde `Error` (ignorar).
    pub fn relay_request_push_token(&self, token: String) -> Result<Vec<u8>, QtError> {
        let ok = token.is_empty()
            || (token.len() <= 4096 && token.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b)));
        if !ok {
            return Err(bad("token"));
        }
        let uid = self.relay_user_id()?;
        json_bytes(serde_json::json!({"RegisterPushToken": {"user_id": uid, "token": token}}))
    }

    /// Premium (0.9.20): ate quando esta conta tem premium. Resposta `Premium { until_ms }`
    /// (0 = sem premium); um relay antigo responde `Error` (tratar como sem premium).
    pub fn relay_request_premium_status(&self) -> Result<Vec<u8>, QtError> {
        json_bytes(serde_json::json!("PremiumStatus"))
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
    /// `nonce` da prova de trabalho desta conta: guardado, ou calculado agora (sem
    /// prender a base de dados durante o calculo).
    fn pow_nonce(&self) -> Result<u64, QtError> {
        let signing = {
            let c = self.lock()?;
            if let Some(n) = prekeys::stored_pow(&c)? {
                return Ok(n);
            }
            st::load_identity(&c)?.identity_signing.verifying_key.to_bytes()
        };
        let n = qt_core::pow::pow_solve(&signing, qt_core::pow::POW_BITS).ok_or(QtError::Crypto)?;
        let c = self.lock()?;
        prekeys::save_pow(&c, n)?;
        Ok(n)
    }

    pub(crate) fn lock(&self) -> Result<std::sync::MutexGuard<'_, st::rusqlite::Connection>, QtError> {
        Ok(relock(&self.conn))
    }

    /// Cifra e avanca o ratchet usando `c` (o CHAMADOR abre e confirma a
    /// transacao: assim pode guardar mais coisas na mesma transacao).
    pub(crate) fn encrypt_with(&self, c: &st::rusqlite::Connection, contact_id: &str, plaintext: &[u8]) -> Result<Vec<u8>, QtError> {
        if plaintext.len() > MAX_PLAINTEXT_LEN {
            return Err(bad("plaintext"));
        }
        let mut state = st::load_session(c, contact_id)?.ok_or(QtError::UnknownSession)?;
        let pending = st::kv_get(c, &format!("pending:{contact_id}"))?;
        // Sessao iniciada antes da 0.9.3 e ainda sem resposta: envelope v2 (cabecalho
        // PQXDH v1, sem enchimento), como quando foi criada. Tudo o resto: v3.
        let (version, kind) = match pending.as_ref().map(|h| h.len()) {
            None => (ENV_VERSION, KIND_MSG),
            Some(HEADER_V2_LEN) => (ENV_VERSION, KIND_PREKEY_MSG_V2),
            Some(HEADER_LEN) => (ENV_VERSION_V2, KIND_PREKEY_MSG),
            Some(_) => return Err(QtError::Storage),
        };
        let msg = if version == ENV_VERSION {
            state.encrypt(&pad_plaintext(plaintext))
        } else {
            state.encrypt(plaintext)
        }
        .map_err(|_| QtError::Crypto)?;
        let mut env = vec![version, kind];
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
        let mut out = vec![SEALED_VERSION];
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
        if sealed_envelope.len() < 3 || sealed_envelope.len() > MAX_ENVELOPE_LEN || sealed_envelope[0] != SEALED_VERSION {
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
        if env.len() < 2 {
            return Err(QtError::InvalidEnvelope);
        }
        let (version, kind) = (env[0], env[1]);
        let header_len = match (version, kind) {
            (ENV_VERSION, KIND_MSG) | (ENV_VERSION_V2, KIND_MSG) => 0,
            (ENV_VERSION, KIND_PREKEY_MSG_V2) => HEADER_V2_LEN,
            (ENV_VERSION_V2, KIND_PREKEY_MSG) => HEADER_LEN,
            _ => return Err(QtError::InvalidEnvelope),
        };
        let padded = version == ENV_VERSION;
        if env.len() < 2 + header_len {
            return Err(QtError::InvalidEnvelope);
        }
        let header = (header_len > 0).then(|| &env[2..2 + header_len]);
        let ratchet_bytes = &env[2 + header_len..];
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
        // `None`: a mensagem foi lida com a sessao do outro lado, mas a nossa fica (ver abaixo).
        let (state, plaintext) = match st::load_session(c, contact_id)? {
            None => {
                let h = header.ok_or(QtError::UnknownSession)?;
                let (mut s, opk) = self.establish_responder(c, contact_id, kind, h)?;
                let p = s.decrypt(&msg).map_err(|_| QtError::Crypto)?;
                used_opk = opk;
                new_session = true;
                (Some(s), p)
            }
            Some(mut s) => match s.decrypt(&msg) {
                Ok(p) => (Some(s), p),
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
                    let (mut ns, opk) = self.establish_responder(c, contact_id, kind, h).map_err(|_| QtError::Crypto)?;
                    let p = ns.decrypt(&msg).map_err(|_| QtError::Crypto)?;
                    let ours_pending = st::kv_get(c, &format!("pending:{contact_id}"))?.is_some();
                    // SESSOES CRUZADAS: os dois iniciaram uma sessao ao mesmo tempo (ex.:
                    // dois membros de um grupo novo). Desempate deterministico e igual nos
                    // dois lados: fica a sessao iniciada por quem tem a chave de identidade
                    // "menor". O outro lado adota-a ao receber a nossa primeira mensagem.
                    let mine = st::load_identity(c)?.identity.public.to_bytes();
                    let theirs = contact_to_identity(contact_id)?.to_bytes();
                    if ours_pending && theirs >= mine {
                        // Ganhamos: a nossa sessao fica. A mensagem dele e autentica (o AEAD
                        // da sessao dele passou), por isso e lida na mesma, sem trocar de
                        // sessao. A OPK (se houver) fica para as mensagens seguintes dele.
                        (None, p)
                    } else {
                        used_opk = opk;
                        new_session = true;
                        replaced = true;
                        (Some(ns), p)
                    }
                }
            },
        };

        // Envelope v3: retira o enchimento. Formato errado = envelope invalido (nada
        // foi guardado ainda).
        let plaintext = if padded { unpad_plaintext(&plaintext).ok_or(QtError::InvalidEnvelope)? } else { plaintext };

        let tx = c.unchecked_transaction()?;
        if let Some(state) = &state {
            st::save_session(&tx, contact_id, state)?;
            st::kv_delete(&tx, &format!("pending:{contact_id}"))?;
        }
        st::record_envelope(&tx, contact_id, &env_id)?;
        if let Some(id) = used_opk {
            st::delete_one_time_prekey(&tx, id)?;
        }
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
            st::kv_delete(&tx, &format!("{}{contact_id}", crate::history::KV_RESTART))?;
        } else if new_session && state.is_some() {
            // Sessao nova recebida depois de o contacto ter terminado a anterior (tipo 10).
            crate::history::after_new_session(self, &tx, contact_id)?;
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
        kind: u8,
        header: &[u8],
    ) -> Result<(RatchetState, Option<u32>), QtError> {
        let their_identity = contact_to_identity(contact_id)?;
        let eph = PublicKey::from(arr32(&header[..32], "header")?);
        let u32_at = |a: usize| -> Result<u32, QtError> {
            Ok(u32::from_be_bytes(header.get(a..a + 4).and_then(|b| b.try_into().ok()).ok_or(QtError::InvalidEnvelope)?))
        };
        let opk_id = u32_at(32)?;
        let pq_id = u32_at(36)?;
        // v2: numero da signed pre-key no cabecalho. v1 (antes da rotacao): a primeira.
        let (spk_id, ct_at) = if kind == KIND_PREKEY_MSG_V2 { (u32_at(40)?, 44) } else { (prekeys::FIRST_ID, 40) };
        let ct = pq_ciphertext_from_bytes(&header[ct_at..]).map_err(|_| QtError::InvalidEnvelope)?;

        let ident = st::load_identity(c)?;
        let spk = prekeys::load_spk(c, &ident, spk_id)?;
        let pq = st::load_pq_prekey(c, pq_id)?.ok_or(QtError::PrekeyUnavailable)?;
        // Le a OPK sem a consumir: so e apagada depois de a 1.a mensagem autenticar.
        let opk = if opk_id == 0 { None } else { Some(st::load_one_time_prekey(c, opk_id)?) };
        let respond = if kind == KIND_PREKEY_MSG_V2 { pqxdh_respond_v2 } else { pqxdh_respond };
        let mut secret = respond(&ident.identity, &spk, &pq, opk.as_ref(), &their_identity, &eph, &ct)
            .map_err(|_| QtError::Crypto)?;
        let state = RatchetState::init_as_responder(secret, spk)
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
    /// Uma parte da copia de recuperacao (`relay_request_backup_get`).
    BackupPart { part: u32, parts: u32, data: Vec<u8> },
    /// Nao ha copia para estas palavras (nunca houve, expirou ou foi apagada).
    BackupNotFound,
    /// Ate quando esta conta tem premium (ms desde 1970); 0 = sem premium.
    Premium { until_ms: u64 },
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
    PreKeyBundle {
        bundle_bytes: Vec<u8>,
        /// One-time prekey do contacto (`[4 numero BE][32 publica]`), se o relay tiver.
        #[serde(default)]
        one_time_pre_key: Option<Vec<u8>>,
    },
    PreKeyBundleNotFound,
    Messages { messages: Vec<WireDelivered> },
    Error { reason: String },
    BlobCreated { blob_id: String, token: String },
    BlobStatus { size: u64, received: u64, complete: bool },
    BlobData { offset: u64, size: u64, data: String },
    BlobNotFound,
    BackupPart { part: u32, parts: u32, data: String },
    BackupNotFound,
    Premium { until_ms: u64 },
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
        // A one-time prekey segue colada ao bundle (`peer_bundle_from_relay_bytes` separa-a):
        // assim a app passa os mesmos bytes de sempre a `start_session`.
        WireServerMsg::PreKeyBundle { mut bundle_bytes, one_time_pre_key } => {
            if let Some(k) = one_time_pre_key.filter(|k| k.len() == OTPK_WIRE_LEN) {
                bundle_bytes.extend_from_slice(&k);
            }
            RelayResponse::Bundle { bytes: bundle_bytes }
        }
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
        WireServerMsg::BackupPart { part, parts, data } => {
            use base64::Engine;
            let data = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| QtError::InvalidEnvelope)?;
            if data.len() > crate::recovery::PART_MAX {
                return Err(QtError::InvalidEnvelope);
            }
            RelayResponse::BackupPart { part, parts, data }
        }
        WireServerMsg::BackupNotFound => RelayResponse::BackupNotFound,
        WireServerMsg::Premium { until_ms } => RelayResponse::Premium { until_ms },
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

/// One-time prekey como o relay a guarda e entrega: `[4 numero BE][32 publica]`.
pub(crate) const OTPK_WIRE_LEN: usize = 36;

/// Interpreta o bundle devolvido pelo relay: bundle v4 (formato de
/// `publishable_bundle`) e, opcionalmente, uma one-time prekey colada no fim. A
/// verificacao criptografica (contra o QT Public ID, assinatura e data) e feita por
/// `start_session`. Bundles v3 (apps ate a 0.9.2) sao recusados: o contacto tem de
/// atualizar a app.
#[uniffi::export]
pub fn peer_bundle_from_relay_bytes(bytes: Vec<u8>) -> Result<PeerBundle, QtError> {
    let (bundle, otpk) = match bytes.len() {
        identity::BUNDLE_V4_LEN => (&bytes[..], None),
        n if n == identity::BUNDLE_V4_LEN + OTPK_WIRE_LEN => {
            (&bytes[..identity::BUNDLE_V4_LEN], Some(&bytes[identity::BUNDLE_V4_LEN..]))
        }
        _ => return Err(QtError::InvalidBundle),
    };
    let b = identity::parse_bundle_v4(bundle).map_err(|_| QtError::InvalidBundle)?;
    let one_time_pre_key = match otpk {
        None => None,
        Some(k) => {
            let id = u32::from_be_bytes(k[..4].try_into().map_err(|_| QtError::InvalidBundle)?);
            (id != 0).then(|| OneTimePreKeyPublic { id, public: k[4..].to_vec() })
        }
    };
    Ok(PeerBundle {
        identity_public: b.identity.to_vec(),
        signing_public: b.signing.to_vec(),
        signed_pre_key_id: b.signed_pre_key_id,
        signed_pre_key_public: b.signed_pre_key.to_vec(),
        created_at: b.created_at,
        signed_pre_key_signature: b.signature.to_vec(),
        pq_pre_key_id: b.pq_pre_key_id,
        pq_pre_key_public: b.pq_pre_key,
        one_time_pre_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> String {
        let p = std::env::temp_dir().join(format!("qt_relock_{tag}_{}.sqlite", std::process::id()));
        let p = p.to_str().unwrap().to_string();
        let _ = std::fs::remove_file(&p);
        p
    }

    /// Regressao (22/10/2026): um panic com a base de dados trancada (o UniFFI apanha-o
    /// e a app continua) envenenava o Mutex e todas as chamadas seguintes davam
    /// `Storage` ate reiniciar a app. Agora a app continua a funcionar e o que estava
    /// a meio da transacao foi desfeito.
    #[test]
    fn panic_com_a_base_de_dados_trancada_nao_bloqueia_a_app() {
        let path = tmp("db");
        let c = QtClient::open(path.clone(), vec![0x5a; 32]).unwrap();
        c.create_identity().unwrap();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let g = c.lock().unwrap();
            let tx = g.unchecked_transaction().unwrap();
            st::kv_set(&tx, "a-meio", b"x").unwrap();
            panic!("panic simulado a meio de uma transacao");
        }));
        assert!(r.is_err());
        assert!(c.conn.is_poisoned(), "o panic envenenou o Mutex");
        // A app continua a funcionar...
        assert!(c.has_identity().unwrap());
        assert!(!c.export_public_identity().unwrap().public_id.is_empty());
        // ...o Mutex ficou limpo e a escrita a meio foi desfeita.
        assert!(!c.conn.is_poisoned());
        assert_eq!(st::kv_get(&c.lock().unwrap(), "a-meio").unwrap(), None);
        drop(c);
        let _ = std::fs::remove_file(&path);
    }

    /// O mesmo para o estado em memoria da presenca e das chamadas.
    #[test]
    fn panic_com_presenca_ou_chamadas_trancadas_nao_bloqueia() {
        let path = tmp("mem");
        let c = QtClient::open(path.clone(), vec![0x5b; 32]).unwrap();
        c.create_identity().unwrap();
        for m in [0, 1] {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if m == 0 {
                    let _g = c.presence.lock().unwrap();
                    panic!("presenca");
                } else {
                    let _g = c.calls.lock().unwrap();
                    panic!("chamadas");
                }
            }));
            assert!(r.is_err());
        }
        assert!(c.presence.is_poisoned() && c.calls.is_poisoned());
        let _p = relock(&c.presence);
        let _k = relock(&c.calls);
        drop((_p, _k));
        assert!(!c.presence.is_poisoned() && !c.calls.is_poisoned());
        // Apagar a identidade (que mexe na presenca) continua a funcionar.
        c.delete_identity().unwrap();
        drop(c);
        let _ = std::fs::remove_file(&path);
    }
}
