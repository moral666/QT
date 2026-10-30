//! Comunidade (0.9.52): a lista das pessoas com medalha (ver `qt_core::community`).
//!
//! ENVIO (so quem tem medalha): a app pergunta [`QtClient::community_submit_request`] ao abrir;
//! so ha pedido quando e preciso: a primeira vez com a medalha (ou depois de recuperar a conta),
//! quando a medalha ou a foto mudam (tirar a foto tambem conta) e, sem mudancas, no maximo uma
//! vez por semana (para o operador nunca ficar sem o envio). Depois do `Ack` do relay a app
//! chama [`QtClient::community_submit_sent`].
//!
//! LISTA (toda a gente, so ao abrir o ecra Comunidade, nada em segundo plano):
//!   1. `relay_request_community_version`: se o numero for o que ja ha (`community_cached_version`),
//!      acabou, nada mais desce;
//!   2. se mudou, `relay_request_community_index` e `community_apply_index`, que confere a
//!      assinatura das medalhas, guarda o indice e devolve so as fotos que ainda nao ha (as de
//!      quem saiu da lista sao apagadas);
//!   3. `relay_request_community_photos` com essas, e `community_put_photo` para cada uma.
//!
//! Sem lista no relay (passaram os 8 dias), a app mostra a copia guardada. Tudo fica na base de
//! dados cifrada da app (SQLCipher), como o resto.

use crate::client::{bad, hex, QtClient, QtError};
use crate::history::now_ms;
use qt_core::{badge as qb, community as qc};
use qt_storage as st;
use sha2::{Digest, Sha256};

/// Sem mudancas, o envio repete-se no maximo uma vez por semana.
pub const RESEND_MS: i64 = 7 * 24 * 3_600_000;
/// Fotos pedidas ao relay de uma vez (o relay manda as que cabem numa pagina).
pub const PHOTOS_PER_REQUEST: usize = 20;

/// O que ja foi enviado: `[32 sha256 do estado][8 ms BE]`.
const KV_SENT: &str = "community:sent";
/// O indice cifrado da ultima lista aceite e o seu numero.
const KV_INDEX: &str = "community:index";
const KV_VERSION: &str = "community:version";
/// Fotos guardadas (hashes em hex, separados por virgulas) e cada foto cifrada.
const KV_PHOTOS: &str = "community:photos";
const KV_PHOTO: &str = "community:photo:";

#[cfg(debug_assertions)]
static TEST_OPERATOR: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();

/// So para testes (builds de debug): outra chave publica do operador.
#[cfg(debug_assertions)]
#[doc(hidden)]
pub fn set_test_operator(public_key: [u8; 32]) {
    let _ = TEST_OPERATOR.set(public_key);
}

fn operator_public() -> [u8; 32] {
    #[cfg(debug_assertions)]
    if let Some(k) = TEST_OPERATOR.get() {
        return *k;
    }
    qt_core::report::OPERATOR_REPORT_PUBLIC
}

/// Um membro da lista, pronto a mostrar: so a medalha e a foto (sem nome nem ID).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CommunityMember {
    /// 1 Founder ... 9 Legacy (como `badge_mine`).
    pub badge: u8,
    /// A foto (WebP ou JPEG) ou `None`: a app mostra o avatar por omissao com a medalha.
    pub photo: Option<Vec<u8>>,
}

/// Uma foto como o relay a manda (cifrada; guardar com `community_put_photo`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CommunityPhotoBlob {
    pub hash: String,
    pub data: Vec<u8>,
}

fn unhex32(h: &str) -> Option<[u8; 32]> {
    if h.len() != 64 {
        return None;
    }
    let v: Option<Vec<u8>> = (0..32).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok()).collect();
    v?.try_into().ok()
}

fn stored_photos(c: &st::rusqlite::Connection) -> Result<Vec<String>, QtError> {
    Ok(st::kv_get(c, KV_PHOTOS)?
        .and_then(|v| String::from_utf8(v).ok())
        .map(|s| s.split(',').filter(|h| h.len() == 64).map(str::to_string).collect())
        .unwrap_or_default())
}

fn set_photos(c: &st::rusqlite::Connection, hashes: &[String]) -> Result<(), QtError> {
    st::kv_set(c, KV_PHOTOS, hashes.join(",").as_bytes())?;
    Ok(())
}

fn cached_index(c: &st::rusqlite::Connection) -> Result<Option<qc::Index>, QtError> {
    let Some(blob) = st::kv_get(c, KV_INDEX)? else { return Ok(None) };
    // Ja foi conferido ao chegar; confere-se outra vez (o ficheiro pode ser de outra versao).
    Ok(qc::open_index_with(&crate::badges::issuer_key(), &qc::COMMUNITY_KEY, &blob).ok())
}

/// O que vai no envio: certificado, foto (ou nenhuma) e o resumo dos dois.
type MyState = (Vec<u8>, Option<Vec<u8>>, [u8; 32]);

/// O estado a enviar e o seu resumo, ou `None` sem medalha valida.
fn my_state(c: &st::rusqlite::Connection) -> Result<Option<MyState>, QtError> {
    let Some(cert) = crate::badges::my_cert(c)? else { return Ok(None) };
    let me = st::load_identity(c)?;
    let ok = crate::badges::verify_cert(&cert)
        .is_some_and(|v| v.badge != qb::BADGE_NONE && v.subject == me.identity.public.to_bytes());
    if !ok {
        return Ok(None);
    }
    let photo = crate::profile::my_photo(c)?.filter(|p| qc::valid_photo(p));
    let mut h = Sha256::new();
    h.update(&cert);
    match &photo {
        Some(p) => {
            h.update([1u8]);
            h.update(p);
        }
        None => h.update([0u8]),
    }
    Ok(Some((cert, photo, h.finalize().into())))
}

#[uniffi::export]
impl QtClient {
    /// ENVIO DA FOTO DA COMUNIDADE: o pedido `CommunitySubmit` para o relay, ou `None` se nao
    /// for preciso (sem medalha, nada mudou e ainda nao passou uma semana, ou a app sem a chave
    /// do operador). Depois do `Ack`, chamar `community_submit_sent`.
    pub fn community_submit_request(&self) -> Result<Option<Vec<u8>>, QtError> {
        let op = operator_public();
        if op == [0u8; 32] {
            return Ok(None);
        }
        let c = self.lock()?;
        let Some((cert, photo, digest)) = my_state(&c)? else { return Ok(None) };
        let now = now_ms();
        if let Some(sent) = st::kv_get(&c, KV_SENT)?.filter(|v| v.len() == 40) {
            let at = i64::from_be_bytes(sent[32..40].try_into().unwrap_or_default());
            if sent[..32] == digest && now - at < RESEND_MS && at <= now {
                return Ok(None);
            }
        }
        let me = st::load_identity(&c)?;
        let sealed = qc::seal_submission_with(&op, &me.identity.private, &cert, now, photo.as_deref())
            .map_err(|_| bad("comunidade"))?;
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(sealed);
        serde_json::to_vec(&serde_json::json!({"CommunitySubmit": {"sealed": b64}}))
            .map(Some)
            .map_err(|_| bad("comunidade"))
    }

    /// O relay aceitou o envio (estado atual): o proximo so quando mudar ou daqui a uma semana.
    pub fn community_submit_sent(&self) -> Result<(), QtError> {
        let c = self.lock()?;
        let Some((_, _, digest)) = my_state(&c)? else { return Ok(()) };
        let mut v = digest.to_vec();
        v.extend_from_slice(&now_ms().to_be_bytes());
        st::kv_set(&c, KV_SENT, &v)?;
        Ok(())
    }

    /// Pedido do numero da lista (o unico pedido quando nada mudou). Resposta `CommunityVersion`.
    pub fn relay_request_community_version(&self) -> Result<Vec<u8>, QtError> {
        Ok(br#""CommunityVersion""#.to_vec())
    }

    /// Pedido do indice. Resposta `CommunityIndex` ou `CommunityNone`.
    pub fn relay_request_community_index(&self) -> Result<Vec<u8>, QtError> {
        Ok(br#""CommunityIndex""#.to_vec())
    }

    /// Pedido de fotos pelo hash (ate `PHOTOS_PER_REQUEST`). Resposta `CommunityPhotos` com as
    /// que couberem: pedir as outras a seguir.
    pub fn relay_request_community_photos(&self, hashes: Vec<String>) -> Result<Vec<u8>, QtError> {
        if hashes.len() > PHOTOS_PER_REQUEST || hashes.iter().any(|h| unhex32(h).is_none()) {
            return Err(bad("hashes"));
        }
        serde_json::to_vec(&serde_json::json!({"CommunityPhotos": {"hashes": hashes}})).map_err(|_| bad("hashes"))
    }

    /// Numero da lista guardada (0 = nenhuma).
    pub fn community_cached_version(&self) -> Result<u64, QtError> {
        let c = self.lock()?;
        Ok(st::kv_get(&c, KV_VERSION)?
            .and_then(|v| v.try_into().ok())
            .map(u64::from_be_bytes)
            .unwrap_or(0))
    }

    /// Indice novo do relay: confere a assinatura das medalhas e o numero (nunca volta para
    /// tras), guarda-o, apaga as fotos de quem saiu e devolve as fotos que faltam descarregar.
    pub fn community_apply_index(&self, version: u64, data: Vec<u8>) -> Result<Vec<String>, QtError> {
        let ix = qc::open_index_with(&crate::badges::issuer_key(), &qc::COMMUNITY_KEY, &data)
            .map_err(|_| bad("comunidade"))?;
        if ix.version != version {
            return Err(bad("comunidade"));
        }
        let c = self.lock()?;
        let current = st::kv_get(&c, KV_VERSION)?.and_then(|v| v.try_into().ok()).map(u64::from_be_bytes).unwrap_or(0);
        if version < current {
            return Err(bad("comunidade"));
        }
        let wanted: Vec<String> = ix.members.iter().filter_map(|m| m.photo_hash.map(|h| hex(&h))).collect();
        let tx = c.unchecked_transaction()?;
        let mut have = stored_photos(&tx)?;
        for h in have.iter().filter(|h| !wanted.contains(h)) {
            st::kv_delete(&tx, &format!("{KV_PHOTO}{h}"))?;
        }
        have.retain(|h| wanted.contains(h));
        set_photos(&tx, &have)?;
        st::kv_set(&tx, KV_INDEX, &data)?;
        st::kv_set(&tx, KV_VERSION, &version.to_be_bytes())?;
        tx.commit()?;
        let mut missing: Vec<String> = wanted.into_iter().filter(|h| !have.contains(h)).collect();
        missing.dedup();
        Ok(missing)
    }

    /// Fotos da lista guardada que ainda faltam (uma descarga interrompida continua da proxima vez).
    pub fn community_missing_photos(&self) -> Result<Vec<String>, QtError> {
        let c = self.lock()?;
        let Some(ix) = cached_index(&c)? else { return Ok(Vec::new()) };
        let have = stored_photos(&c)?;
        let mut out: Vec<String> =
            ix.members.iter().filter_map(|m| m.photo_hash.map(|h| hex(&h))).filter(|h| !have.contains(h)).collect();
        out.dedup();
        Ok(out)
    }

    /// Uma foto do relay: so e guardada se for de um membro da lista e o hash bater certo.
    pub fn community_put_photo(&self, hash: String, data: Vec<u8>) -> Result<bool, QtError> {
        let h = unhex32(&hash).ok_or_else(|| bad("hash"))?;
        if qc::open_photo(&h, &data).is_err() {
            return Ok(false);
        }
        let c = self.lock()?;
        let Some(ix) = cached_index(&c)? else { return Ok(false) };
        if !ix.members.iter().any(|m| m.photo_hash == Some(h)) {
            return Ok(false);
        }
        let tx = c.unchecked_transaction()?;
        let mut have = stored_photos(&tx)?;
        if !have.contains(&hash) {
            have.push(hash.clone());
            set_photos(&tx, &have)?;
        }
        st::kv_set(&tx, &format!("{KV_PHOTO}{hash}"), &data)?;
        tx.commit()?;
        Ok(true)
    }

    /// Os membros da lista guardada, pela ordem das medalhas e, dentro de cada uma, de entrada.
    /// Vazia sem lista.
    pub fn community_members(&self) -> Result<Vec<CommunityMember>, QtError> {
        let c = self.lock()?;
        let Some(mut ix) = cached_index(&c)? else { return Ok(Vec::new()) };
        qc::sort_members(&mut ix.members);
        let mut out = Vec::with_capacity(ix.members.len());
        for m in ix.members {
            let photo = match m.photo_hash {
                Some(h) => st::kv_get(&c, &format!("{KV_PHOTO}{}", hex(&h)))?.and_then(|b| qc::open_photo(&h, &b).ok()),
                None => None,
            };
            out.push(CommunityMember { badge: m.badge, photo });
        }
        Ok(out)
    }
}
