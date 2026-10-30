//! Badges (aro colorido a volta da foto): certificado assinado pelo emissor (ver
//! `qt_core::badge`) que segue, cifrado, para os MEUS contactos (tipo 13, dentro do
//! Double Ratchet, como a foto de perfil). O relay ve um envelope normal: nao sabe que
//! e um badge e nao guarda nada.
//!
//!   * O emissor (ferramenta `badge`, no computador do operador) envia o certificado a pessoa como
//!     uma mensagem normal. A app guarda-o se for valido e for para a SUA identidade.
//!   * A app envia o seu certificado aos contactos (`badge_sync`, uma vez por versao).
//!     Quem recebe so o aceita se a assinatura for do emissor e o certificado for da
//!     identidade de quem o enviou (nao se pode copiar o badge de outra pessoa).
//!   * Vale sempre o certificado mais recente: retirar um badge e um certificado novo
//!     com o badge 0, que tambem segue para os contactos.
//!
//! FORMATO: payload v1, tipo 13, corpo = o certificado (106 bytes). `msg_id` fixo.

use crate::client::{
    bad, contact_to_identity, hex, parse_public_id, public_identity, QtClient, QtError,
};
use crate::history::{is_blocked_in, new_id, now_ms, ReceiveResult, PAYLOAD_V1};
use qt_core::badge::{self as qb, BadgeCert};
use qt_storage as st;
use qt_storage::messages as hm;

pub(crate) const T_BADGE: u8 = 13;
/// `msg_id` sempre igual: uma app antiga (sem o tipo 13) mostra "atualiza a app" no
/// maximo uma vez por contacto, como na foto de perfil.
const MSG_ID: [u8; 16] = *b"QT-BADGE-v1\0\0\0\0\0";

fn encode(cert: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 1 + 16 + 8 + cert.len());
    out.push(PAYLOAD_V1);
    out.push(T_BADGE);
    out.extend_from_slice(&MSG_ID);
    out.extend_from_slice(&now_ms().to_be_bytes());
    out.extend_from_slice(cert);
    out
}

/// O meu certificado (o mais recente que recebi do emissor).
const KV_MINE: &str = "badge:mine";
/// Versao do meu estado (ms em que o certificado chegou).
const KV_MY_VER: &str = "badge:my_ver";
/// Versao do meu estado que ja foi para este contacto.
const KV_SENT: &str = "badge:sent:";
/// Certificado recebido deste contacto.
const KV_OF: &str = "badge:of:";

/// Recuperacao da conta: o meu certificado (a copia so traz os que eram validos), que volta
/// a seguir para os contactos (versao nova).
pub(crate) fn restore_mine(c: &st::rusqlite::Connection, cert: &[u8]) -> Result<(), QtError> {
    st::kv_set(c, KV_MINE, cert)?;
    st::kv_set(c, KV_MY_VER, &now_ms().to_be_bytes())?;
    Ok(())
}

/// O meu certificado, para a copia de recuperacao.
pub(crate) fn my_cert(c: &st::rusqlite::Connection) -> Result<Option<Vec<u8>>, QtError> {
    Ok(st::kv_get(c, KV_MINE)?)
}

#[cfg(debug_assertions)]
static TEST_ISSUER: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();

/// So para testes (builds de debug): usa outra chave do emissor.
#[cfg(debug_assertions)]
#[doc(hidden)]
pub fn set_test_issuer(public_key: [u8; 32]) {
    let _ = TEST_ISSUER.set(public_key);
}

fn verify(cert: &[u8]) -> Option<BadgeCert> {
    qb::verify_with(&issuer_key(), cert)
}

/// Chave publica do emissor (a fixa; nos testes de debug, a de `set_test_issuer`). Tambem
/// assina a lista da Comunidade.
pub(crate) fn issuer_key() -> [u8; 32] {
    #[cfg(debug_assertions)]
    if let Some(k) = TEST_ISSUER.get() {
        return *k;
    }
    qb::ISSUER_PUBLIC_KEY
}

pub(crate) fn verify_cert(cert: &[u8]) -> Option<BadgeCert> {
    verify(cert)
}

fn get_i64(c: &st::rusqlite::Connection, key: &str) -> Result<Option<i64>, QtError> {
    Ok(st::kv_get(c, key)?
        .and_then(|v| v.try_into().ok())
        .map(i64::from_be_bytes))
}

/// Certificado guardado (ja foi verificado ao chegar; volta-se a verificar por seguranca).
fn stored(c: &st::rusqlite::Connection, key: &str) -> Result<Option<BadgeCert>, QtError> {
    Ok(st::kv_get(c, key)?.and_then(|b| verify(&b)))
}

fn my_identity(c: &st::rusqlite::Connection) -> Result<[u8; 32], QtError> {
    let me = public_identity(&st::load_identity(c)?);
    Ok(parse_public_id(&me.public_id)?.0)
}

/// Envia o meu certificado aos contactos que ainda nao tem esta versao.
fn sync_in(
    client: &QtClient,
    c: &st::rusqlite::Connection,
    contact_ids: &[String],
) -> Result<u32, QtError> {
    let Some(ver) = get_i64(c, KV_MY_VER)? else {
        return Ok(0);
    };
    let Some(cert) = st::kv_get(c, KV_MINE)? else {
        return Ok(0);
    };
    let none = verify(&cert).is_none_or(|v| v.badge == qb::BADGE_NONE);
    let mut n = 0;
    for cid in contact_ids {
        if contact_to_identity(cid).is_err()
            || is_blocked_in(c, cid)?
            || st::load_session(c, cid)?.is_none()
        {
            continue;
        }
        let key = format!("{KV_SENT}{cid}");
        let sent = get_i64(c, &key)?;
        // Ja tem esta versao; ou nunca lhe mandei badge nenhum e agora nao tenho (nada a dizer).
        if sent == Some(ver) || (sent.is_none() && none) {
            continue;
        }
        let blob = client.seal_with(c, cid, &encode(&cert))?;
        hm::outbox_put(c, &new_id()?, cid, hm::OUTBOX_RECEIPT, &blob)?;
        st::kv_set(c, &key, &ver.to_be_bytes())?;
        n += 1;
    }
    Ok(n)
}

/// Tipo 13 recebido de `cid` (ja decifrado, dentro da transacao da rececao).
pub(crate) fn on_badge(
    c: &st::rusqlite::Connection,
    cid: &str,
    body: &[u8],
) -> Result<ReceiveResult, QtError> {
    let no_change = || {
        Ok(ReceiveResult::NoChange {
            contact_id: cid.to_string(),
        })
    };
    let Some(cert) = verify(body) else {
        return no_change();
    };
    let (key, mine) = if cert.subject == my_identity(c)? {
        // Do emissor para mim (vem de quem quer que seja: so a assinatura conta).
        (KV_MINE.to_string(), true)
    } else if hex(&cert.subject) == cid {
        // O badge do proprio contacto.
        (format!("{KV_OF}{cid}"), false)
    } else {
        return no_change();
    };
    if stored(c, &key)?.is_some_and(|old| old.issued_at >= cert.issued_at) {
        return no_change();
    }
    st::kv_set(c, &key, body)?;
    if mine {
        st::kv_set(c, KV_MY_VER, &now_ms().to_be_bytes())?;
    }
    Ok(ReceiveResult::Updated {
        contact_id: cid.to_string(),
    })
}

/// Sessao nova com `cid` (ele reinstalou e recuperou a conta, ou reiniciou a sessao): ja nao
/// tem o meu badge. Esquece o que lhe enviei para o proximo `badge_sync` o mandar outra vez
/// (0.9.29; antes so voltava a seguir com um badge novo).
pub(crate) fn forget_sent(c: &st::rusqlite::Connection, cid: &str) -> Result<(), QtError> {
    st::kv_delete(c, &format!("{KV_SENT}{cid}"))?;
    Ok(())
}

/// Ao apagar um contacto: esquece o badge dele e o que lhe enviei.
pub(crate) fn forget_contact(c: &st::rusqlite::Connection, cid: &str) -> Result<(), QtError> {
    st::kv_delete(c, &format!("{KV_OF}{cid}"))?;
    st::kv_delete(c, &format!("{KV_SENT}{cid}"))?;
    Ok(())
}

#[uniffi::export]
impl QtClient {
    /// BADGES: envia o meu badge a quem ainda nao tem a versao atual (os `contact_ids`
    /// sao os contactos guardados). Chamar com a sincronizacao da foto de perfil.
    pub fn badge_sync(&self, contact_ids: Vec<String>) -> Result<u32, QtError> {
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        let n = sync_in(self, &tx, &contact_ids)?;
        tx.commit()?;
        Ok(n)
    }

    /// O meu badge: 0 = nenhum, 1 Founder, 2 Co-founder, 3 Dev, 4 Tester, 5 Contributor.
    pub fn badge_mine(&self) -> Result<u8, QtError> {
        let c = self.lock()?;
        let me = my_identity(&c)?;
        Ok(stored(&c, KV_MINE)?
            .filter(|v| v.subject == me)
            .map_or(qb::BADGE_NONE, |v| v.badge))
    }

    /// Badge deste contacto (0 = nenhum).
    pub fn badge_of(&self, contact_id: String) -> Result<u8, QtError> {
        let c = self.lock()?;
        let v = stored(&c, &format!("{KV_OF}{contact_id}"))?;
        Ok(v.filter(|v| hex(&v.subject) == contact_id)
            .map_or(qb::BADGE_NONE, |v| v.badge))
    }

    /// So para a ferramenta do emissor: poe na outbox o certificado para `contact_id`
    /// (tem de haver sessao). Recusa certificados invalidos ou de outra identidade.
    pub fn badge_deliver(&self, contact_id: String, certificate: Vec<u8>) -> Result<(), QtError> {
        match verify(&certificate) {
            Some(v) if hex(&v.subject) == contact_id => {}
            _ => return Err(bad("certificate")),
        }
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        let blob = self.seal_with(&tx, &contact_id, &encode(&certificate))?;
        hm::outbox_put(&tx, &new_id()?, &contact_id, hm::OUTBOX_RECEIPT, &blob)?;
        tx.commit()?;
        Ok(())
    }
}
