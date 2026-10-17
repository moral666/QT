//! Anexos (imagens e GIFs ate 25 MiB), cifrados ponta a ponta.
//!
//! ENVIAR
//!   1. `attachment_create`: o ficheiro (ja comprimido pela app) e cifrado com
//!      uma chave NOVA e aleatoria, so dele, e guardado na pasta de anexos. A
//!      mensagem aparece logo no historico ("a enviar").
//!   2. A app pede ao relay espaco (`blob_create_request`), guarda o id
//!      (`attachment_set_blob`) e envia os blocos (`blob_put_request`). Se a rede
//!      falhar, `blob_info_request` diz onde retomar.
//!   3. `attachment_uploaded`: a chave, o id no relay, o tamanho e o tipo seguem
//!      numa mensagem normal do Double Ratchet (com Sealed Sender), pela outbox.
//!
//! RECEBER
//!   1. `receive_sealed` guarda a mensagem do anexo ("a descarregar").
//!   2. A app descarrega os blocos (`blob_get_request` + `attachment_write_chunk`)
//!      e chama `attachment_download_finished`, que verifica o ficheiro inteiro.
//!   3. A app pede ao relay que apague o ficheiro (`blob_delete_request`).
//!
//! CIFRA DO FICHEIRO
//!   O conteudo e arredondado para cima (Padme: no maximo ~12% a mais) para o
//!   relay nao saber o tamanho exato, e cifrado em blocos de 40 KiB com
//!   ChaCha20-Poly1305: nonce = numero do bloco + marca de "ultimo bloco". O
//!   relay nao consegue trocar, cortar nem reordenar blocos sem a verificacao
//!   falhar. Cada bloco cifrado tem exatamente o tamanho de um pedido ao relay.
//!
//!   Os ficheiros ficam cifrados no telemovel; a chave vive na base de dados
//!   SQLCipher. So `attachment_read` devolve a imagem em claro, para memoria.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::client::{bad, contact_to_identity, hex, unhex, QtClient, QtError};
use crate::history::{to_chat, ChatMessage, OutboxItem};
use qt_storage as st;
use qt_storage::messages as hm;

/// Maior imagem/GIF aceite (antes de cifrar).
pub const MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;
/// Bytes cifrados por bloco (= por pedido ao relay; igual a `qt_server::blobs::BLOB_CHUNK`).
pub const BLOB_CHUNK: usize = 40 * 1024;
const TAG: usize = 16;
const PLAIN_CHUNK: usize = BLOB_CHUNK - TAG;
const AAD: &[u8] = b"QT-ATT-v1";

/// Tipos aceites. Tudo o resto e recusado (nao se envia nem se mostra).
pub const MIME_TYPES: [&str; 4] = ["image/jpeg", "image/png", "image/gif", "image/webp"];

// Estado guardado nos metadados (`st`).
const ST_UPLOADING: u8 = 0;
const ST_READY: u8 = 1;
const ST_DOWNLOADING: u8 = 2;
const ST_FAILED: u8 = 3;
const ST_EXPIRED: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AttachmentState {
    /// A enviar para o relay (ainda nao saiu a mensagem com a chave).
    Uploading,
    /// Recebido mas ainda por descarregar.
    Downloading,
    /// Pronto a mostrar.
    Ready,
    /// O envio ou a descarga falhou de vez (ex.: ficheiro corrompido).
    Failed,
    /// O relay ja apagou o ficheiro (passaram 24 h) antes de ser descarregado.
    Expired,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AttachmentInfo {
    pub mime: String,
    /// Tamanho da imagem em bytes (antes de cifrar).
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub state: AttachmentState,
    /// Bytes cifrados (o que viaja pelo relay). Para mostrar o progresso.
    pub transfer_size: u64,
}

/// Um anexo por enviar ou por descarregar.
#[derive(Debug, Clone, uniffi::Record)]
pub struct AttachmentRef {
    pub contact_id: String,
    pub message_id: String,
}

/// Metadados guardados no `body` da mensagem (dentro do SQLCipher).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct AttMeta {
    pub v: u8,
    pub key: String,
    #[serde(default)]
    pub blob: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    pub size: u64,
    pub clen: u64,
    pub mime: String,
    pub w: u32,
    pub h: u32,
    pub st: u8,
}

impl AttMeta {
    pub(crate) fn parse(body: &str) -> Option<AttMeta> {
        serde_json::from_str(body).ok()
    }

    fn to_body(&self) -> Result<String, QtError> {
        serde_json::to_string(self).map_err(|_| QtError::Storage)
    }

    pub(crate) fn info(&self) -> AttachmentInfo {
        AttachmentInfo {
            mime: self.mime.clone(),
            size: self.size,
            width: self.w,
            height: self.h,
            state: match self.st {
                ST_UPLOADING => AttachmentState::Uploading,
                ST_READY => AttachmentState::Ready,
                ST_DOWNLOADING => AttachmentState::Downloading,
                ST_EXPIRED => AttachmentState::Expired,
                _ => AttachmentState::Failed,
            },
            transfer_size: self.clen,
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        if self.mime == "image/gif" {
            "GIF"
        } else {
            "📷 Foto"
        }
    }

    fn key(&self) -> Result<[u8; 32], QtError> {
        let k = unhex(&self.key).ok_or(QtError::Storage)?;
        k.as_slice().try_into().map_err(|_| QtError::Storage)
    }
}

// ------------------------------------------------------------------ cifra

/// Padme (Nikitin et al., 2019): arredonda o tamanho para esconder o valor
/// exato, com no maximo ~12% a mais.
pub(crate) fn padme(len: u64) -> u64 {
    if len < 2 {
        return len.max(1);
    }
    let e = 63 - len.leading_zeros() as u64; // floor(log2 len)
    let s = 64 - e.leading_zeros() as u64; // floor(log2 e) + 1
    let last_bits = e - s;
    let mask = (1u64 << last_bits) - 1;
    (len + mask) & !mask
}

/// Bytes cifrados de um anexo com `size` bytes.
pub(crate) fn cipher_len(size: u64) -> u64 {
    let padded = padme(size);
    let chunks = padded.div_ceil(PLAIN_CHUNK as u64);
    padded + chunks * TAG as u64
}

fn nonce(index: u64, last: bool) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..8].copy_from_slice(&index.to_be_bytes());
    n[11] = last as u8;
    n
}

/// Cifra `plain` (ja em memoria) para `out`.
fn encrypt_to(plain: &[u8], key: &[u8; 32], out: &mut impl Write) -> Result<u64, QtError> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let padded_len = padme(plain.len() as u64) as usize;
    let chunks = padded_len.div_ceil(PLAIN_CHUNK);
    let mut buf = vec![0u8; PLAIN_CHUNK];
    let mut written = 0u64;
    for i in 0..chunks {
        let start = i * PLAIN_CHUNK;
        let end = (start + PLAIN_CHUNK).min(padded_len);
        let n = end - start;
        // Copia a parte real e completa com zeros (arredondamento).
        buf[..n].fill(0);
        if start < plain.len() {
            let real_end = end.min(plain.len());
            buf[..real_end - start].copy_from_slice(&plain[start..real_end]);
        }
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce(i as u64, i + 1 == chunks)), Payload { msg: &buf[..n], aad: AAD })
            .map_err(|_| QtError::Crypto)?;
        out.write_all(&ct).map_err(|_| QtError::Storage)?;
        written += ct.len() as u64;
    }
    buf.zeroize();
    Ok(written)
}

/// Decifra um anexo inteiro e devolve os `size` bytes originais. Falha se
/// faltar, sobrar ou tiver sido mexido um unico byte.
fn decrypt_all(ct: &[u8], key: &[u8; 32], size: u64) -> Result<Vec<u8>, QtError> {
    if ct.len() as u64 != cipher_len(size) {
        return Err(QtError::Crypto);
    }
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let chunks = ct.len().div_ceil(BLOB_CHUNK);
    let mut out = Vec::with_capacity(ct.len());
    for (i, block) in ct.chunks(BLOB_CHUNK).enumerate() {
        let p = cipher
            .decrypt(Nonce::from_slice(&nonce(i as u64, i + 1 == chunks)), Payload { msg: block, aad: AAD })
            .map_err(|_| QtError::Crypto)?;
        out.extend_from_slice(&p);
    }
    out.truncate(size as usize);
    Ok(out)
}

// ------------------------------------------------------- mensagem no ratchet

/// Corpo do tipo "anexo" (vai DENTRO do Double Ratchet):
///   [1 versao=1][32 chave][16 id no relay][8 tamanho BE][8 tamanho cifrado BE]
///   [4 largura BE][4 altura BE][1 n][n bytes do tipo, ex.: image/gif]
pub(crate) fn encode_wire(m: &AttMeta) -> Result<Vec<u8>, QtError> {
    let key = m.key()?;
    let blob = m.blob.as_deref().and_then(unhex).filter(|b| b.len() == 16).ok_or(QtError::Storage)?;
    let mut out = Vec::with_capacity(80);
    out.push(1);
    out.extend_from_slice(&key);
    out.extend_from_slice(&blob);
    out.extend_from_slice(&m.size.to_be_bytes());
    out.extend_from_slice(&m.clen.to_be_bytes());
    out.extend_from_slice(&m.w.to_be_bytes());
    out.extend_from_slice(&m.h.to_be_bytes());
    out.push(m.mime.len() as u8);
    out.extend_from_slice(m.mime.as_bytes());
    Ok(out)
}

/// Interpreta o corpo recebido. `None` = malformado ou com valores invalidos.
pub(crate) fn decode_wire(b: &[u8]) -> Option<AttMeta> {
    const FIXED: usize = 1 + 32 + 16 + 8 + 8 + 4 + 4 + 1;
    if b.len() < FIXED || b[0] != 1 {
        return None;
    }
    let mlen = b[FIXED - 1] as usize;
    if b.len() != FIXED + mlen {
        return None;
    }
    let mime = std::str::from_utf8(&b[FIXED..]).ok()?;
    if !MIME_TYPES.contains(&mime) {
        return None;
    }
    let size = u64::from_be_bytes(b[49..57].try_into().ok()?);
    let clen = u64::from_be_bytes(b[57..65].try_into().ok()?);
    if size == 0 || size > MAX_ATTACHMENT_BYTES || clen != cipher_len(size) {
        return None;
    }
    Some(AttMeta {
        v: 1,
        key: hex(&b[1..33]),
        blob: Some(hex(&b[33..49])),
        token: None,
        size,
        clen,
        mime: mime.to_string(),
        w: u32::from_be_bytes(b[65..69].try_into().ok()?).min(100_000),
        h: u32::from_be_bytes(b[69..73].try_into().ok()?).min(100_000),
        st: ST_DOWNLOADING,
    })
}

/// Metadados de um anexo recebido, prontos a guardar.
pub(crate) fn received_body(m: &AttMeta) -> Result<String, QtError> {
    m.to_body()
}

// ---------------------------------------------------------------- ficheiros

fn parse_id(s: &str) -> Result<Vec<u8>, QtError> {
    match unhex(s) {
        Some(b) if b.len() == 16 => Ok(b),
        _ => Err(bad("message_id")),
    }
}

impl QtClient {
    /// Ficheiro cifrado de um anexo. O nome nao revela o contacto nem o id.
    pub(crate) fn attachment_path(&self, contact_id: &str, msg_id: &[u8]) -> PathBuf {
        let mut h = Sha256::new();
        h.update(b"QT-ATT-FILE");
        h.update(contact_id.as_bytes());
        h.update(msg_id);
        self.files_dir.join(format!("{}.bin", &hex(&h.finalize())[..32]))
    }

    fn ensure_files_dir(&self) -> Result<(), QtError> {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        b.create(&self.files_dir).map_err(|_| QtError::Storage)
    }

    /// Apaga os ficheiros dos anexos de um contacto (chamar ANTES de apagar as linhas).
    pub(crate) fn remove_contact_files(&self, c: &st::rusqlite::Connection, contact_id: &str) -> Result<(), QtError> {
        for id in hm::attachment_ids(c, contact_id)? {
            let _ = std::fs::remove_file(self.attachment_path(contact_id, &id));
        }
        Ok(())
    }

    fn load_meta(&self, c: &st::rusqlite::Connection, contact_id: &str, id: &[u8]) -> Result<(hm::StoredMessage, AttMeta), QtError> {
        let m = hm::get_message(c, contact_id, id)?.ok_or_else(|| bad("message_id"))?;
        if m.kind != hm::KIND_ATTACHMENT {
            return Err(bad("message_id"));
        }
        let meta = AttMeta::parse(&m.body).ok_or(QtError::Storage)?;
        Ok((m, meta))
    }

    fn meta_of(&self, contact_id: &str, message_id: &str) -> Result<(Vec<u8>, AttMeta), QtError> {
        let id = parse_id(message_id)?;
        let c = self.lock()?;
        let (_, meta) = self.load_meta(&c, contact_id, &id)?;
        Ok((id, meta))
    }

    fn set_state(&self, contact_id: &str, id: &[u8], f: impl FnOnce(&mut AttMeta)) -> Result<ChatMessage, QtError> {
        let c = self.lock()?;
        let (_, mut meta) = self.load_meta(&c, contact_id, id)?;
        f(&mut meta);
        hm::update_body(&c, contact_id, id, &meta.to_body()?)?;
        let m = hm::get_message(&c, contact_id, id)?.ok_or(QtError::Storage)?;
        Ok(to_chat(&m))
    }
}

fn read_at(path: &Path, offset: u64, len: usize) -> Result<Vec<u8>, QtError> {
    let mut f = std::fs::File::open(path).map_err(|_| QtError::Storage)?;
    f.seek(SeekFrom::Start(offset)).map_err(|_| QtError::Storage)?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).map_err(|_| QtError::Storage)?;
    Ok(buf)
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn json_bytes(v: serde_json::Value) -> Result<Vec<u8>, QtError> {
    serde_json::to_vec(&v).map_err(|_| QtError::InvalidEnvelope)
}

#[uniffi::export]
impl QtClient {
    /// Prepara uma imagem/GIF para enviar a `contact_id`. `source_path` e um
    /// ficheiro temporario da app (ja comprimido; pode ser apagado a seguir).
    /// A mensagem fica no historico como "a enviar".
    pub fn attachment_create(
        &self,
        contact_id: String,
        source_path: String,
        mime: String,
        width: u32,
        height: u32,
    ) -> Result<ChatMessage, QtError> {
        let _ = contact_to_identity(&contact_id)?;
        if !MIME_TYPES.contains(&mime.as_str()) {
            return Err(bad("mime"));
        }
        let size = std::fs::metadata(&source_path).map_err(|_| bad("source_path"))?.len();
        if size == 0 || size > MAX_ATTACHMENT_BYTES {
            return Err(bad("size"));
        }
        let mut plain = std::fs::read(&source_path).map_err(|_| bad("source_path"))?;
        let mut key = [0u8; 32];
        getrandom::getrandom(&mut key).map_err(|_| QtError::Crypto)?;
        let mut id = [0u8; 16];
        getrandom::getrandom(&mut id).map_err(|_| QtError::Crypto)?;

        self.ensure_files_dir()?;
        let path = self.attachment_path(&contact_id, &id);
        let tmp = path.with_extension("tmp");
        let res = (|| -> Result<u64, QtError> {
            let mut f = std::fs::File::create(&tmp).map_err(|_| QtError::Storage)?;
            let n = encrypt_to(&plain, &key, &mut f)?;
            f.sync_all().map_err(|_| QtError::Storage)?;
            std::fs::rename(&tmp, &path).map_err(|_| QtError::Storage)?;
            Ok(n)
        })();
        plain.zeroize();
        let clen = match res {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                key.zeroize();
                return Err(e);
            }
        };
        debug_assert_eq!(clen, cipher_len(size));
        let meta = AttMeta {
            v: 1,
            key: hex(&key),
            blob: None,
            token: None,
            size,
            clen,
            mime,
            w: width.min(100_000),
            h: height.min(100_000),
            st: ST_UPLOADING,
        };
        key.zeroize();
        let now = crate::history::now_ms();
        let c = self.lock()?;
        let inserted = hm::insert_message(&c, &id, &contact_id, true, hm::KIND_ATTACHMENT, &meta.to_body()?, now, now, hm::ST_PENDING);
        if inserted.is_err() {
            let _ = std::fs::remove_file(&path);
        }
        inserted?;
        let m = hm::get_message(&c, &contact_id, &id)?.ok_or(QtError::Storage)?;
        Ok(to_chat(&m))
    }

    /// Anexos meus ainda por enviar ao relay (retomar depois de falhas/reinicios).
    pub fn attachments_pending_upload(&self) -> Result<Vec<AttachmentRef>, QtError> {
        let c = self.lock()?;
        Ok(hm::attachments_matching(&c, true, &format!("\"st\":{ST_UPLOADING}"))?
            .into_iter()
            .map(|(contact_id, id)| AttachmentRef { contact_id, message_id: hex(&id) })
            .collect())
    }

    /// Anexos recebidos ainda por descarregar.
    pub fn attachments_pending_download(&self) -> Result<Vec<AttachmentRef>, QtError> {
        let c = self.lock()?;
        Ok(hm::attachments_matching(&c, false, &format!("\"st\":{ST_DOWNLOADING}"))?
            .into_iter()
            .map(|(contact_id, id)| AttachmentRef { contact_id, message_id: hex(&id) })
            .collect())
    }

    /// Ja tem espaco reservado no relay? (`None` = pedir com `blob_create_request`.)
    pub fn attachment_blob_id(&self, contact_id: String, message_id: String) -> Result<Option<String>, QtError> {
        Ok(self.meta_of(&contact_id, &message_id)?.1.blob)
    }

    /// Pedido ao relay: reservar espaco para este anexo.
    pub fn blob_create_request(&self, contact_id: String, message_id: String) -> Result<Vec<u8>, QtError> {
        let (_, meta) = self.meta_of(&contact_id, &message_id)?;
        json_bytes(serde_json::json!({"BlobCreate": {"size": meta.clen}}))
    }

    /// Guarda o id e o token devolvidos pelo relay (`RelayResponse::BlobCreated`).
    pub fn attachment_set_blob(&self, contact_id: String, message_id: String, blob_id: String, token: String) -> Result<(), QtError> {
        let ok = |s: &str| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
        if !ok(&blob_id) || !ok(&token) {
            return Err(bad("blob_id"));
        }
        let id = parse_id(&message_id)?;
        self.set_state(&contact_id, &id, |m| {
            m.blob = Some(blob_id);
            m.token = Some(token);
        })?;
        Ok(())
    }

    /// Pedido ao relay: quantos bytes deste anexo ja chegaram (para retomar).
    pub fn blob_info_request(&self, contact_id: String, message_id: String) -> Result<Vec<u8>, QtError> {
        let (_, meta) = self.meta_of(&contact_id, &message_id)?;
        let blob = meta.blob.ok_or_else(|| bad("blob_id"))?;
        json_bytes(serde_json::json!({"BlobInfo": {"blob_id": blob}}))
    }

    /// Pedido ao relay: o bloco que comeca em `offset` (multiplo de 40 KiB).
    pub fn blob_put_request(&self, contact_id: String, message_id: String, offset: u64) -> Result<Vec<u8>, QtError> {
        let (id, meta) = self.meta_of(&contact_id, &message_id)?;
        let (blob, token) = match (meta.blob, meta.token) {
            (Some(b), Some(t)) => (b, t),
            _ => return Err(bad("blob_id")),
        };
        let chunk = BLOB_CHUNK as u64;
        if offset >= meta.clen || offset / chunk * chunk != offset {
            return Err(bad("offset"));
        }
        let len = (meta.clen - offset).min(BLOB_CHUNK as u64) as usize;
        let data = read_at(&self.attachment_path(&contact_id, &id), offset, len)?;
        json_bytes(serde_json::json!({"BlobPut": {
            "blob_id": blob, "token": token, "offset": offset, "data": b64().encode(data),
        }}))
    }

    /// O relay ja tem o ficheiro inteiro: envia a mensagem com a chave (cifrada
    /// pelo Double Ratchet) pela outbox. A sessao com o contacto tem de existir.
    pub fn attachment_uploaded(&self, contact_id: String, message_id: String) -> Result<OutboxItem, QtError> {
        let id = parse_id(&message_id)?;
        let idarr: [u8; 16] = id.as_slice().try_into().map_err(|_| bad("message_id"))?;
        let c = self.lock()?;
        let (m, mut meta) = self.load_meta(&c, &contact_id, &id)?;
        if !m.outgoing || meta.blob.is_none() {
            return Err(bad("message_id"));
        }
        let tx = c.unchecked_transaction()?;
        crate::history::queue_intro_if_needed(self, &tx, &contact_id)?;
        let plain = crate::history::encode_attachment(&idarr, m.sent_at, &encode_wire(&meta)?);
        let blob = self.seal_with(&tx, &contact_id, &plain)?;
        meta.st = ST_READY;
        meta.token = None; // ja nao e preciso escrever mais
        hm::update_body(&tx, &contact_id, &id, &meta.to_body()?)?;
        hm::outbox_put(&tx, &id, &contact_id, hm::OUTBOX_MESSAGE, &blob)?;
        tx.commit()?;
        Ok(OutboxItem { outbox_id: message_id, contact_id, blob })
    }

    /// Pedido ao relay: o bloco cifrado que comeca em `offset`.
    pub fn blob_get_request(&self, contact_id: String, message_id: String, offset: u64) -> Result<Vec<u8>, QtError> {
        let (_, meta) = self.meta_of(&contact_id, &message_id)?;
        let blob = meta.blob.ok_or_else(|| bad("blob_id"))?;
        json_bytes(serde_json::json!({"BlobGet": {"blob_id": blob, "offset": offset}}))
    }

    /// Bytes ja descarregados deste anexo (para retomar uma descarga).
    pub fn attachment_downloaded_bytes(&self, contact_id: String, message_id: String) -> Result<u64, QtError> {
        let id = parse_id(&message_id)?;
        Ok(std::fs::metadata(self.attachment_path(&contact_id, &id)).map(|m| m.len()).unwrap_or(0))
    }

    /// Guarda um bloco descarregado (`RelayResponse::BlobData`). Devolve quantos
    /// bytes ja estao no telemovel.
    pub fn attachment_write_chunk(&self, contact_id: String, message_id: String, offset: u64, data: Vec<u8>) -> Result<u64, QtError> {
        let (id, meta) = self.meta_of(&contact_id, &message_id)?;
        if meta.st != ST_DOWNLOADING || data.is_empty() || data.len() > BLOB_CHUNK {
            return Err(bad("data"));
        }
        let end = offset.checked_add(data.len() as u64).ok_or_else(|| bad("offset"))?;
        if end > meta.clen {
            return Err(bad("offset"));
        }
        self.ensure_files_dir()?;
        let path = self.attachment_path(&contact_id, &id);
        let current = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if end <= current {
            return Ok(current); // bloco repetido
        }
        if offset != current {
            return Err(bad("offset"));
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path).map_err(|_| QtError::Storage)?;
        f.write_all(&data).map_err(|_| QtError::Storage)?;
        if end == meta.clen {
            f.sync_all().map_err(|_| QtError::Storage)?;
        }
        Ok(end)
    }

    /// A descarga acabou: verifica o ficheiro inteiro (todos os blocos, a ordem
    /// e o fim). Se estiver bem fica "pronto"; se nao, "falhou" e e apagado.
    pub fn attachment_download_finished(&self, contact_id: String, message_id: String) -> Result<ChatMessage, QtError> {
        let (id, meta) = self.meta_of(&contact_id, &message_id)?;
        let path = self.attachment_path(&contact_id, &id);
        let ok = match std::fs::read(&path) {
            Ok(ct) => {
                let key = meta.key()?;
                decrypt_all(&ct, &key, meta.size).map(|mut p| p.zeroize()).is_ok()
            }
            Err(_) => false,
        };
        if !ok {
            let _ = std::fs::remove_file(&path);
        }
        self.set_state(&contact_id, &id, |m| m.st = if ok { ST_READY } else { ST_FAILED })
    }

    /// O envio ou a descarga falhou de vez (`expired`: o relay ja nao tem o ficheiro).
    pub fn attachment_failed(&self, contact_id: String, message_id: String, expired: bool) -> Result<ChatMessage, QtError> {
        let id = parse_id(&message_id)?;
        let msg = self.set_state(&contact_id, &id, |m| {
            m.st = if expired { ST_EXPIRED } else { ST_FAILED };
            m.token = None;
        })?;
        if !msg.outgoing {
            let _ = std::fs::remove_file(self.attachment_path(&contact_id, &id));
        }
        Ok(msg)
    }

    /// Pedido ao relay: apagar o ficheiro (depois de descarregado e verificado).
    pub fn blob_delete_request(&self, contact_id: String, message_id: String) -> Result<Vec<u8>, QtError> {
        let (_, meta) = self.meta_of(&contact_id, &message_id)?;
        let blob = meta.blob.ok_or_else(|| bad("blob_id"))?;
        json_bytes(serde_json::json!({"BlobDelete": {"blob_id": blob}}))
    }

    /// A imagem em claro, para mostrar (so em memoria; nunca escrita em disco).
    pub fn attachment_read(&self, contact_id: String, message_id: String) -> Result<Vec<u8>, QtError> {
        let id = parse_id(&message_id)?;
        let (m, meta) = {
            let c = self.lock()?;
            self.load_meta(&c, &contact_id, &id)?
        };
        // As minhas (o ficheiro existe desde o inicio) e as recebidas ja verificadas.
        if !(m.outgoing || meta.st == ST_READY) {
            return Err(bad("message_id"));
        }
        let ct = std::fs::read(self.attachment_path(&contact_id, &id)).map_err(|_| QtError::Storage)?;
        decrypt_all(&ct, &meta.key()?, meta.size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padme_arredonda_pouco_e_sempre_para_cima() {
        for len in [1u64, 2, 3, 100, 1000, 40_944, 300_000, 5_000_000, MAX_ATTACHMENT_BYTES] {
            let p = padme(len);
            assert!(p >= len, "{len}");
            assert!((p - len) as f64 <= len as f64 * 0.12 + 1.0, "{len} -> {p}");
        }
        // Tamanhos parecidos caem no mesmo valor (e isso que esconde o tamanho exato).
        assert_eq!(padme(5_000_000), padme(5_000_100));
    }

    fn roundtrip(len: usize) {
        let mut plain = vec![0u8; len];
        getrandom::getrandom(&mut plain).unwrap();
        let key = [7u8; 32];
        let mut ct = Vec::new();
        let n = encrypt_to(&plain, &key, &mut ct).unwrap();
        assert_eq!(n, ct.len() as u64);
        assert_eq!(n, cipher_len(len as u64), "{len}");
        assert_eq!(decrypt_all(&ct, &key, len as u64).unwrap(), plain, "{len}");
    }

    #[test]
    fn cifra_por_blocos_ida_e_volta() {
        for len in [1, 15, PLAIN_CHUNK - 1, PLAIN_CHUNK, PLAIN_CHUNK + 1, 3 * PLAIN_CHUNK + 77, 1_000_000] {
            roundtrip(len);
        }
    }

    #[test]
    fn qualquer_alteracao_ao_ficheiro_e_detetada() {
        let plain = vec![42u8; 3 * PLAIN_CHUNK + 10];
        let key = [9u8; 32];
        let mut ct = Vec::new();
        encrypt_to(&plain, &key, &mut ct).unwrap();
        let size = plain.len() as u64;
        // Um byte trocado.
        let mut bad = ct.clone();
        bad[BLOB_CHUNK + 5] ^= 1;
        assert!(decrypt_all(&bad, &key, size).is_err());
        // Blocos trocados de ordem.
        let mut swapped = ct[BLOB_CHUNK..2 * BLOB_CHUNK].to_vec();
        swapped.extend_from_slice(&ct[..BLOB_CHUNK]);
        swapped.extend_from_slice(&ct[2 * BLOB_CHUNK..]);
        assert!(decrypt_all(&swapped, &key, size).is_err());
        // Cortado no fim de um bloco (o ultimo passaria a ser outro).
        assert!(decrypt_all(&ct[..3 * BLOB_CHUNK], &key, size).is_err());
        // Chave errada.
        assert!(decrypt_all(&ct, &[8u8; 32], size).is_err());
        // Tamanho declarado diferente.
        assert!(decrypt_all(&ct, &key, size + 50_000).is_err());
    }

    #[test]
    fn corpo_no_ratchet_ida_e_volta_e_valores_invalidos() {
        let m = AttMeta {
            v: 1,
            key: hex(&[1u8; 32]),
            blob: Some(hex(&[2u8; 16])),
            token: Some("x".into()),
            size: 123_456,
            clen: cipher_len(123_456),
            mime: "image/gif".into(),
            w: 480,
            h: 270,
            st: ST_READY,
        };
        let w = encode_wire(&m).unwrap();
        let back = decode_wire(&w).unwrap();
        assert_eq!((back.key, back.blob, back.size, back.clen, back.mime, back.w, back.h), (m.key.clone(), m.blob.clone(), m.size, m.clen, m.mime.clone(), 480, 270));
        assert_eq!(back.st, ST_DOWNLOADING);
        assert_eq!(back.token, None, "o token de escrita nunca vai para o contacto");

        let mut bad_mime = w.clone();
        let l = bad_mime.len();
        bad_mime[l - 1] = b'x'; // image/gix
        assert!(decode_wire(&bad_mime).is_none());
        let mut too_big = m.clone();
        too_big.size = MAX_ATTACHMENT_BYTES + 1;
        too_big.clen = cipher_len(too_big.size);
        assert!(decode_wire(&encode_wire(&too_big).unwrap()).is_none());
        let mut wrong_clen = m.clone();
        wrong_clen.clen += 1;
        assert!(decode_wire(&encode_wire(&wrong_clen).unwrap()).is_none());
        assert!(decode_wire(&w[..w.len() - 1]).is_none());
    }
}
