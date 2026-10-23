//! Foto de perfil: cada pessoa escolhe uma foto e ela segue, cifrada, para os SEUS
//! contactos (tipo 12, dentro do Double Ratchet, como a presenca). O relay ve um
//! envelope 1 a 1 do tamanho maximo normal (10 240): nao sabe que e uma foto.
//!
//!   * A app reduz a foto (no maximo 3 MB a original) para um quadrado pequeno em WebP
//!     ou JPEG que caiba em `MAX_PHOTO_BYTES`. Aqui so se confere o formato e o tamanho.
//!   * So se pode trocar de foto de `CHANGE_GAP_MS` em `CHANGE_GAP_MS` (72 h). Tirar a
//!     foto pode-se sempre, mas nao reinicia o prazo. Quem recebe tambem nao aceita
//!     uma foto nova do mesmo contacto antes de `RECV_GAP_MS` (71 h, folga para
//!     relogios), por isso uma app alterada nao consegue mudar mais depressa.
//!   * So para contactos (a lista que a app passa): `profile_photo_sync` envia o estado
//!     atual a quem ainda nao o tem (contactos novos, sessoes refeitas), uma so vez por
//!     versao. Fotos recebidas guardam-se na base de dados cifrada (kv) e apagam-se com
//!     o contacto.
//!
//! FORMATO: payload v1, tipo 12, corpo `[1 op][foto]`: op 1 = foto (WebP ou JPEG),
//! op 0 = sem foto (corpo so com o op). O `msg_id` e sempre o mesmo (`MSG_ID`): uma
//! versao antiga da app, que nao conhece o tipo 12, mostra o aviso "atualiza a app"
//! no maximo uma vez por contacto.

use crate::client::{bad, contact_to_identity, QtClient, QtError};
use crate::history::{is_blocked_in, now_ms, ReceiveResult, PAYLOAD_V1};
use qt_storage as st;
use qt_storage::messages as hm;

pub(crate) const T_PROFILE: u8 = 12;
/// Cabecalho do payload do tipo 12 antes da foto: versao, tipo, msg_id, sent_at e op.
const HEADER_BYTES: usize = 1 + 1 + 16 + 8 + 1;
/// Tamanho maximo da foto ja reduzida: o payload inteiro (cabecalho + foto) tem de caber
/// em `MAX_PLAINTEXT_LEN` (10 000). Ate a 0.9.10 o limite era 10 000 so para a foto, e
/// uma foto entre 9 974 e 10 000 bytes era recusada ao cifrar ("plaintext").
pub const MAX_PHOTO_BYTES: usize = crate::client::MAX_PLAINTEXT_LEN - HEADER_BYTES;
/// Intervalo minimo entre duas fotos novas (72 h).
pub const CHANGE_GAP_MS: i64 = 72 * 3_600_000;
/// Do lado de quem recebe: 71 h (uma hora de folga para relogios diferentes).
const RECV_GAP_MS: i64 = 71 * 3_600_000;
const MSG_ID: [u8; 16] = *b"QT-PROFILE-v1\0\0\0";
const OP_NONE: u8 = 0;
const OP_PHOTO: u8 = 1;

/// A minha foto (bytes); nao existe = sem foto.
const KV_MY_PHOTO: &str = "profile:my_photo";
/// Quando escolhi a ultima foto (ms, para as 72 h).
const KV_MY_SET_AT: &str = "profile:my_set_at";
/// Versao do meu estado (ms da ultima mudanca: foto nova ou retirada).
const KV_MY_VER: &str = "profile:my_ver";
/// Versao do meu estado que ja foi para este contacto.
const KV_SENT: &str = "profile:sent:";
/// Foto recebida deste contacto.
const KV_PHOTO: &str = "profile:photo:";
/// Quando aceitei a ultima foto deste contacto (hora local, ms).
const KV_PHOTO_AT: &str = "profile:photo_at:";

fn get_i64(c: &st::rusqlite::Connection, key: &str) -> Result<Option<i64>, QtError> {
    Ok(st::kv_get(c, key)?.and_then(|v| v.try_into().ok()).map(i64::from_be_bytes))
}

/// WebP (`RIFF....WEBP`) ou JPEG (`FF D8 FF`), com tamanho dentro do limite.
pub(crate) fn valid_photo(p: &[u8]) -> bool {
    let webp = p.len() >= 12 && &p[..4] == b"RIFF" && &p[8..12] == b"WEBP";
    let jpeg = p.len() >= 3 && p[..3] == [0xFF, 0xD8, 0xFF];
    (webp || jpeg) && p.len() <= MAX_PHOTO_BYTES
}

fn encode(photo: Option<&[u8]>, now: i64) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 1 + 16 + 8 + 1 + photo.map_or(0, |p| p.len()));
    out.push(PAYLOAD_V1);
    out.push(T_PROFILE);
    out.extend_from_slice(&MSG_ID);
    out.extend_from_slice(&now.to_be_bytes());
    match photo {
        Some(p) => {
            out.push(OP_PHOTO);
            out.extend_from_slice(p);
        }
        None => out.push(OP_NONE),
    }
    out
}

/// Corpo do tipo 12 recebido: `Some(Some(foto))`, `Some(None)` (sem foto) ou `None` (invalido).
pub(crate) fn decode_body(body: &[u8]) -> Option<Option<Vec<u8>>> {
    match body.split_first() {
        Some((&OP_NONE, [])) => Some(None),
        Some((&OP_PHOTO, p)) if valid_photo(p) => Some(Some(p.to_vec())),
        _ => None,
    }
}

/// Envia o meu estado atual aos contactos que ainda nao tem esta versao.
fn sync_in(client: &QtClient, c: &st::rusqlite::Connection, contact_ids: &[String]) -> Result<u32, QtError> {
    let Some(ver) = get_i64(c, KV_MY_VER)? else { return Ok(0) };
    let photo = st::kv_get(c, KV_MY_PHOTO)?;
    let mut n = 0;
    for cid in contact_ids {
        if contact_to_identity(cid).is_err() || is_blocked_in(c, cid)? || st::load_session(c, cid)?.is_none() {
            continue;
        }
        let key = format!("{KV_SENT}{cid}");
        let sent = get_i64(c, &key)?;
        // Ja tem esta versao; ou nunca lhe mandei nada e nao tenho foto (nada a dizer).
        if sent == Some(ver) || (sent.is_none() && photo.is_none()) {
            continue;
        }
        let blob = client.seal_with(c, cid, &encode(photo.as_deref(), now_ms()))?;
        hm::outbox_put(c, &crate::history::new_id()?, cid, hm::OUTBOX_RECEIPT, &blob)?;
        st::kv_set(c, &key, &ver.to_be_bytes())?;
        n += 1;
    }
    Ok(n)
}

/// Tipo 12 recebido de `cid` (ja decifrado, dentro da transacao da rececao).
pub(crate) fn on_profile(c: &st::rusqlite::Connection, cid: &str, body: &[u8]) -> Result<ReceiveResult, QtError> {
    let no_change = || Ok(ReceiveResult::NoChange { contact_id: cid.to_string() });
    let updated = || Ok(ReceiveResult::Updated { contact_id: cid.to_string() });
    if is_blocked_in(c, cid)? {
        return no_change();
    }
    let Some(photo) = decode_body(body) else { return no_change() };
    let key = format!("{KV_PHOTO}{cid}");
    let old = st::kv_get(c, &key)?;
    match photo {
        None => {
            if old.is_none() {
                return no_change();
            }
            st::kv_delete(c, &key)?;
            updated()
        }
        Some(p) => {
            if old.as_deref() == Some(&p[..]) {
                return no_change();
            }
            let at_key = format!("{KV_PHOTO_AT}{cid}");
            let now = now_ms();
            if get_i64(c, &at_key)?.is_some_and(|t| now - t < RECV_GAP_MS && t <= now) {
                return no_change();
            }
            st::kv_set(c, &key, &p)?;
            st::kv_set(c, &at_key, &now.to_be_bytes())?;
            updated()
        }
    }
}

/// Sessao nova com `cid`: ja nao tem a minha foto. O proximo `profile_photo_sync` manda-a outra
/// vez (0.9.29).
pub(crate) fn forget_sent(c: &st::rusqlite::Connection, cid: &str) -> Result<(), QtError> {
    st::kv_delete(c, &format!("{KV_SENT}{cid}"))?;
    Ok(())
}

/// Ao apagar um contacto: esquece a foto dele e o que lhe enviei.
pub(crate) fn forget_contact(c: &st::rusqlite::Connection, cid: &str) -> Result<(), QtError> {
    st::kv_delete(c, &format!("{KV_PHOTO}{cid}"))?;
    st::kv_delete(c, &format!("{KV_PHOTO_AT}{cid}"))?;
    st::kv_delete(c, &format!("{KV_SENT}{cid}"))?;
    Ok(())
}

#[uniffi::export]
impl QtClient {
    /// FOTO DE PERFIL: a minha foto, ja reduzida pela app (WebP ou JPEG, ate
    /// `MAX_PHOTO_BYTES`). Envia-a aos `contact_ids` (os contactos guardados). Erro
    /// `InvalidInput { field: "photo_wait" }` se ainda nao passaram 72 h desde a
    /// ultima foto (ver `profile_photo_next_change_ms`); `"photo"` se o formato ou o
    /// tamanho nao servirem. Devolve quantos envelopes ficaram na outbox.
    pub fn profile_photo_set(&self, photo: Vec<u8>, contact_ids: Vec<String>) -> Result<u32, QtError> {
        if !valid_photo(&photo) {
            return Err(bad("photo"));
        }
        let c = self.lock()?;
        let now = now_ms();
        if get_i64(&c, KV_MY_SET_AT)?.is_some_and(|t| now - t < CHANGE_GAP_MS && t <= now) {
            return Err(bad("photo_wait"));
        }
        let tx = c.unchecked_transaction()?;
        st::kv_set(&tx, KV_MY_PHOTO, &photo)?;
        st::kv_set(&tx, KV_MY_SET_AT, &now.to_be_bytes())?;
        st::kv_set(&tx, KV_MY_VER, &now.to_be_bytes())?;
        let n = sync_in(self, &tx, &contact_ids)?;
        tx.commit()?;
        Ok(n)
    }

    /// Tira a minha foto (pode-se sempre; nao reinicia as 72 h). Avisa quem a tinha.
    pub fn profile_photo_remove(&self, contact_ids: Vec<String>) -> Result<u32, QtError> {
        let c = self.lock()?;
        if st::kv_get(&c, KV_MY_PHOTO)?.is_none() {
            return Ok(0);
        }
        let tx = c.unchecked_transaction()?;
        st::kv_delete(&tx, KV_MY_PHOTO)?;
        st::kv_set(&tx, KV_MY_VER, &now_ms().to_be_bytes())?;
        let n = sync_in(self, &tx, &contact_ids)?;
        tx.commit()?;
        Ok(n)
    }

    /// Envia o meu estado (foto ou "sem foto") a quem ainda nao tem a versao atual.
    /// Chamar ao abrir a app e quando a lista de contactos mudar.
    pub fn profile_photo_sync(&self, contact_ids: Vec<String>) -> Result<u32, QtError> {
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        let n = sync_in(self, &tx, &contact_ids)?;
        tx.commit()?;
        Ok(n)
    }

    /// A minha foto (None = sem foto).
    pub fn profile_photo_mine(&self) -> Result<Option<Vec<u8>>, QtError> {
        let c = self.lock()?;
        Ok(st::kv_get(&c, KV_MY_PHOTO)?)
    }

    /// Daqui a quantos ms posso escolher uma foto nova (0 = ja).
    pub fn profile_photo_next_change_ms(&self) -> Result<i64, QtError> {
        let c = self.lock()?;
        let now = now_ms();
        Ok(match get_i64(&c, KV_MY_SET_AT)? {
            Some(t) if t <= now => (t + CHANGE_GAP_MS - now).max(0),
            _ => 0,
        })
    }

    /// Foto recebida deste contacto (None = sem foto).
    pub fn profile_photo_of(&self, contact_id: String) -> Result<Option<Vec<u8>>, QtError> {
        let c = self.lock()?;
        Ok(st::kv_get(&c, &format!("{KV_PHOTO}{contact_id}"))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formato_do_corpo() {
        let webp = [b"RIFF".as_slice(), &[0, 0, 0, 0], b"WEBPVP8 "].concat();
        assert!(valid_photo(&webp));
        assert!(valid_photo(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(!valid_photo(b"GIF89a"));
        assert!(!valid_photo(&[[0xFF, 0xD8, 0xFF].as_slice(), &vec![0; MAX_PHOTO_BYTES]].concat()));
        assert_eq!(decode_body(&[OP_NONE]), Some(None));
        assert_eq!(decode_body(&[OP_NONE, 1]), None);
        assert_eq!(decode_body(&[[OP_PHOTO].as_slice(), &webp].concat()), Some(Some(webp.clone())));
        let p = encode(Some(&webp), 7);
        assert_eq!((p[0], p[1], &p[2..18]), (PAYLOAD_V1, T_PROFILE, &MSG_ID[..]));
        // Uma foto do tamanho maximo cabe no maior balde do enchimento.
        // A foto maior que se aceita cabe no limite do plaintext (antes nao cabia).
        assert_eq!(encode(Some(&vec![0; MAX_PHOTO_BYTES]), 0).len(), crate::client::MAX_PLAINTEXT_LEN);
    }
}
