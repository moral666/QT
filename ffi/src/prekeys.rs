//! Chaves publicadas no relay: rotacao da signed pre-key e da prekey ML-KEM,
//! one-time prekeys (uso unico) e a prova de trabalho da conta. Desde a app 0.9.3.
//!
//! Tudo acontece dentro das chamadas que a app ja fazia (`publishable_bundle` e
//! `relay_request_register_bundle`), por isso o Kotlin nao muda:
//!
//! - **Rotacao (7 dias):** ao publicar, se a signed pre-key tiver mais de 7 dias, e
//!   trocada por uma nova (com numero seguinte), e o mesmo para a prekey ML-KEM. As
//!   anteriores ficam guardadas para responder a primeiras mensagens ainda a caminho
//!   (as ultimas `KEEP_PREVIOUS`, cerca de 35 dias).
//! - **One-time prekeys:** uma vez por dia, o pedido de registo leva um lote novo de
//!   `OPK_BATCH` chaves publicas, que substitui as que o relay tinha. O relay entrega
//!   uma a cada pedido de bundle e apaga-a. As privadas ficam no storage ate serem
//!   usadas, ou ate `KEEP_OPK_SECS` depois do lote.
//! - **Prova de trabalho:** calculada uma vez (uns segundos) e guardada; vai em todos
//!   os registos, e o relay so a verifica quando a conta e nova (ver `qt_core::pow`).

use crate::client::QtError;
use qt_core::primitives::DhKeyPair;
use qt_storage as st;
use x25519_dalek::StaticSecret;

const KV_SPK_ID: &str = "spk_id";
const KV_SPK_TS: &str = "spk_ts";
const KV_SPK_OLD: &str = "spk_old:";
const KV_PQ_ID: &str = "pq_id";
const KV_OPK_UPLOAD: &str = "opk_upload_ts";
const KV_OPK_BATCHES: &str = "opk_batches";
pub(crate) const KV_OPK_NEXT: &str = "opk_next";
const KV_POW: &str = "pow_nonce";

/// Numero da signed pre-key e da prekey PQ antes da rotacao existir (apps ate a 0.9.2).
pub(crate) const FIRST_ID: u32 = 1;
/// Idade a partir da qual a signed pre-key e a prekey PQ sao trocadas.
pub const ROTATE_SECS: u64 = 7 * 86_400;
/// Quantas signed pre-keys / prekeys PQ anteriores se guardam (~35 dias com rotacao semanal).
pub const KEEP_PREVIOUS: u32 = 5;
/// One-time prekeys por lote e de quanto em quanto tempo se envia um lote novo.
pub const OPK_BATCH: u32 = 100;
pub const OPK_UPLOAD_EVERY_SECS: u64 = 86_400;
/// Durante quanto tempo se guardam as privadas de um lote (fila do relay: 30 dias).
pub const KEEP_OPK_SECS: u64 = 35 * 86_400;
/// Bundle de um contacto: mais antigo do que isto (ou no futuro) e recusado.
pub const MAX_BUNDLE_AGE_SECS: u64 = 90 * 86_400;
pub const MAX_BUNDLE_FUTURE_SECS: u64 = 86_400;

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn get_u32(c: &st::rusqlite::Connection, k: &str) -> Result<Option<u32>, QtError> {
    match st::kv_get(c, k)? {
        None => Ok(None),
        Some(b) => Ok(Some(u32::from_be_bytes(b.as_slice().try_into().map_err(|_| QtError::Storage)?))),
    }
}

fn get_u64(c: &st::rusqlite::Connection, k: &str) -> Result<Option<u64>, QtError> {
    match st::kv_get(c, k)? {
        None => Ok(None),
        Some(b) => Ok(Some(u64::from_be_bytes(b.as_slice().try_into().map_err(|_| QtError::Storage)?))),
    }
}

/// Numero da signed pre-key atual (a que esta em `StoredIdentity`).
pub(crate) fn current_spk_id(c: &st::rusqlite::Connection) -> Result<u32, QtError> {
    Ok(get_u32(c, KV_SPK_ID)?.unwrap_or(FIRST_ID))
}

/// Numero da prekey ML-KEM atual.
pub(crate) fn current_pq_id(c: &st::rusqlite::Connection) -> Result<u32, QtError> {
    Ok(get_u32(c, KV_PQ_ID)?.unwrap_or(FIRST_ID))
}

/// Hora de criacao da signed pre-key atual (assinada no bundle). Identidades de antes
/// da 0.9.3 nao a tem: conta a partir de agora.
fn spk_created(c: &st::rusqlite::Connection) -> Result<u64, QtError> {
    if let Some(t) = get_u64(c, KV_SPK_TS)? {
        return Ok(t);
    }
    let t = now();
    st::kv_set(c, KV_SPK_TS, &t.to_be_bytes())?;
    Ok(t)
}

/// A signed pre-key com o numero `id`: a atual ou uma anterior ainda guardada.
pub(crate) fn load_spk(c: &st::rusqlite::Connection, ident: &st::StoredIdentity, id: u32) -> Result<DhKeyPair, QtError> {
    if id == current_spk_id(c)? {
        return Ok(ident.signed_pre_key.clone());
    }
    let b = st::kv_get(c, &format!("{KV_SPK_OLD}{id}"))?.ok_or(QtError::PrekeyUnavailable)?;
    let raw: [u8; 32] = b.get(..32).and_then(|s| s.try_into().ok()).ok_or(QtError::Storage)?;
    let private = StaticSecret::from(raw);
    let public = x25519_dalek::PublicKey::from(&private);
    Ok(DhKeyPair { private, public })
}

/// Troca a signed pre-key e a prekey ML-KEM se tiverem mais de `ROTATE_SECS`.
/// Devolve (numero da spk, hora de criacao da spk, numero da prekey PQ) atuais.
/// Corre dentro da transacao do chamador.
pub(crate) fn rotate_if_due(c: &st::rusqlite::Connection, ident: &mut st::StoredIdentity) -> Result<(u32, u64, u32), QtError> {
    let mut spk_id = current_spk_id(c)?;
    let mut created = spk_created(c)?;
    let mut pq_id = current_pq_id(c)?;
    let t = now();
    if t.saturating_sub(created) >= ROTATE_SECS {
        // Guarda a anterior (privada + hora) para primeiras mensagens ainda a caminho.
        let mut old = ident.signed_pre_key.private.to_bytes().to_vec();
        old.extend_from_slice(&created.to_be_bytes());
        st::kv_set(c, &format!("{KV_SPK_OLD}{spk_id}"), &old)?;
        zeroize::Zeroize::zeroize(&mut old);
        ident.signed_pre_key = DhKeyPair::generate();
        st::save_identity(c, ident)?;
        spk_id = spk_id.checked_add(1).ok_or(QtError::Storage)?;
        created = t;
        st::kv_set(c, KV_SPK_ID, &spk_id.to_be_bytes())?;
        st::kv_set(c, KV_SPK_TS, &created.to_be_bytes())?;
        if let Some(gone) = spk_id.checked_sub(KEEP_PREVIOUS + 1) {
            st::kv_delete(c, &format!("{KV_SPK_OLD}{gone}"))?;
        }

        let new_pq = st::max_pq_prekey_id(c)?.max(pq_id).checked_add(1).ok_or(QtError::Storage)?;
        st::save_pq_prekey(c, new_pq, &qt_core::pqxdh::PqPreKeyPair::generate())?;
        st::kv_set(c, KV_PQ_ID, &new_pq.to_be_bytes())?;
        pq_id = new_pq;
        if let Some(gone) = pq_id.checked_sub(KEEP_PREVIOUS + 1) {
            if gone >= 1 {
                st::delete_pq_prekey(c, gone)?;
            }
        }
    }
    Ok((spk_id, created, pq_id))
}

/// Gera `count` one-time prekeys novas (numeros nunca reutilizados), guarda as
/// privadas e devolve (numero, publica).
pub(crate) fn new_one_time_prekeys(c: &st::rusqlite::Connection, count: u32) -> Result<Vec<(u32, [u8; 32])>, QtError> {
    let mut next = get_u32(c, KV_OPK_NEXT)?.unwrap_or(1);
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let kp = DhKeyPair::generate();
        st::save_one_time_prekey(c, next, &kp)?;
        out.push((next, kp.public.to_bytes()));
        next = next.checked_add(1).ok_or(QtError::Storage)?;
    }
    st::kv_set(c, KV_OPK_NEXT, &next.to_be_bytes())?;
    Ok(out)
}

/// Se passou um dia desde o ultimo lote, gera um lote novo para enviar ao relay
/// (`[4 numero BE][32 publica]` por chave) e apaga as privadas dos lotes com mais de
/// `KEEP_OPK_SECS`. `None`: ainda nao e altura.
pub(crate) fn one_time_prekeys_to_upload(c: &st::rusqlite::Connection) -> Result<Option<Vec<Vec<u8>>>, QtError> {
    let t = now();
    if let Some(last) = get_u64(c, KV_OPK_UPLOAD)? {
        if t.saturating_sub(last) < OPK_UPLOAD_EVERY_SECS {
            return Ok(None);
        }
    }
    let keys = new_one_time_prekeys(c, OPK_BATCH)?;
    let start = keys.first().map(|k| k.0).unwrap_or(1);
    // Lotes: [4 primeiro numero BE][8 hora BE] por lote, do mais antigo ao mais novo.
    let mut batches: Vec<(u32, u64)> = st::kv_get(c, KV_OPK_BATCHES)?
        .unwrap_or_default()
        .chunks_exact(12)
        .map(|b| (u32::from_be_bytes([b[0], b[1], b[2], b[3]]), u64::from_be_bytes(b[4..12].try_into().unwrap_or([0; 8]))))
        .collect();
    batches.push((start, t));
    batches.retain(|(_, at)| t.saturating_sub(*at) < KEEP_OPK_SECS);
    if let Some((oldest, _)) = batches.first() {
        st::delete_one_time_prekeys_below(c, *oldest)?;
    }
    let packed: Vec<u8> = batches.iter().flat_map(|(s, at)| s.to_be_bytes().into_iter().chain(at.to_be_bytes())).collect();
    st::kv_set(c, KV_OPK_BATCHES, &packed)?;
    st::kv_set(c, KV_OPK_UPLOAD, &t.to_be_bytes())?;
    Ok(Some(
        keys.into_iter()
            .map(|(id, public)| {
                let mut v = id.to_be_bytes().to_vec();
                v.extend_from_slice(&public);
                v
            })
            .collect(),
    ))
}

/// `nonce` da prova de trabalho guardado (se ja foi calculado).
pub(crate) fn stored_pow(c: &st::rusqlite::Connection) -> Result<Option<u64>, QtError> {
    get_u64(c, KV_POW)
}

pub(crate) fn save_pow(c: &st::rusqlite::Connection, nonce: u64) -> Result<(), QtError> {
    st::kv_set(c, KV_POW, &nonce.to_be_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{peer_bundle_from_relay_bytes, QtClient};
    use std::sync::Arc;

    fn client(name: &str) -> (Arc<QtClient>, String) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let i = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let p = format!("/tmp/qt_prekeys_{name}_{}_{i}.sqlite", std::process::id());
        let _ = std::fs::remove_file(&p);
        let c = QtClient::open(p.clone(), vec![0x5a; 32]).unwrap();
        c.create_identity().unwrap();
        (c, p)
    }

    /// Faz de conta que a signed pre-key atual foi criada ha `secs` segundos.
    fn age_spk(c: &QtClient, secs: u64) {
        let conn = c.lock().unwrap();
        st::kv_set(&conn, KV_SPK_TS, &(now() - secs).to_be_bytes()).unwrap();
    }

    fn json(req: &[u8]) -> serde_json::Value {
        serde_json::from_slice(req).unwrap()
    }

    #[test]
    fn roda_ao_fim_de_7_dias_e_a_anterior_ainda_responde() {
        let (alice, _) = client("rot_a");
        let (bob, _) = client("rot_b");
        let b1 = peer_bundle_from_relay_bytes(bob.publishable_bundle().unwrap()).unwrap();
        assert_eq!((b1.signed_pre_key_id, b1.pq_pre_key_id), (FIRST_ID, FIRST_ID));
        // A Alice inicia com o bundle antigo; entretanto o Bob roda as chaves.
        let bob_pid = bob.export_public_identity().unwrap().public_id;
        let cid = alice.start_session(bob_pid, b1).unwrap();
        let env = alice.encrypt(cid, b"ainda com a chave antiga".to_vec()).unwrap();
        age_spk(&bob, ROTATE_SECS + 10);
        let b2 = peer_bundle_from_relay_bytes(bob.publishable_bundle().unwrap()).unwrap();
        assert_eq!((b2.signed_pre_key_id, b2.pq_pre_key_id), (2, 2));
        // Publicar outra vez logo a seguir nao volta a rodar.
        let b3 = peer_bundle_from_relay_bytes(bob.publishable_bundle().unwrap()).unwrap();
        assert_eq!(b3.signed_pre_key_id, 2);
        let alice_cid = crate::client::hex(&alice.export_public_identity().unwrap().identity_public);
        assert_eq!(bob.decrypt(alice_cid, env).unwrap(), b"ainda com a chave antiga");
    }

    #[test]
    fn guarda_so_as_ultimas_chaves_anteriores() {
        let (bob, _) = client("rot_keep");
        for _ in 0..(KEEP_PREVIOUS + 3) {
            age_spk(&bob, ROTATE_SECS + 10);
            bob.publishable_bundle().unwrap();
        }
        let conn = bob.lock().unwrap();
        let cur = current_spk_id(&conn).unwrap();
        assert_eq!(cur, KEEP_PREVIOUS + 4);
        let ident = st::load_identity(&conn).unwrap();
        for id in (cur - KEEP_PREVIOUS)..cur {
            assert!(load_spk(&conn, &ident, id).is_ok(), "spk {id} devia existir");
        }
        assert!(matches!(load_spk(&conn, &ident, cur - KEEP_PREVIOUS - 1), Err(QtError::PrekeyUnavailable)));
        assert!(st::load_pq_prekey(&conn, current_pq_id(&conn).unwrap() - KEEP_PREVIOUS - 1).unwrap().is_none());
    }

    #[test]
    fn registo_leva_prova_de_trabalho_e_um_lote_por_dia() {
        let (c, _) = client("reg");
        let bundle = c.publishable_bundle().unwrap();
        let r1 = json(&c.relay_request_register_bundle(bundle.clone()).unwrap());
        let body = &r1["RegisterPreKeyBundle"];
        let pow = body["pow"].as_u64().unwrap();
        let signing: [u8; 32] = c.export_public_identity().unwrap().signing_public.try_into().unwrap();
        assert!(qt_core::pow::pow_verify(&signing, pow, qt_core::pow::POW_BITS));
        let keys = body["one_time_pre_keys"].as_array().unwrap();
        assert_eq!(keys.len(), OPK_BATCH as usize);
        assert!(keys.iter().all(|k| k.as_array().unwrap().len() == 36));
        // No mesmo dia: a mesma prova (guardada), sem lote novo.
        let r2 = json(&c.relay_request_register_bundle(bundle).unwrap());
        assert_eq!(r2["RegisterPreKeyBundle"]["pow"].as_u64(), Some(pow));
        assert!(r2["RegisterPreKeyBundle"].get("one_time_pre_keys").is_none());
    }

    #[test]
    fn lotes_antigos_sao_apagados() {
        let (c, _) = client("opk_old");
        let conn = c.lock().unwrap();
        let first = one_time_prekeys_to_upload(&conn).unwrap().unwrap();
        let first_id = u32::from_be_bytes(first[0][..4].try_into().unwrap());
        // O lote tem mais de 35 dias e ja passou um dia desde o envio.
        let old = now() - KEEP_OPK_SECS - 10;
        let mut b = first_id.to_be_bytes().to_vec();
        b.extend_from_slice(&old.to_be_bytes());
        st::kv_set(&conn, KV_OPK_BATCHES, &b).unwrap();
        st::kv_set(&conn, KV_OPK_UPLOAD, &old.to_be_bytes()).unwrap();
        assert!(one_time_prekeys_to_upload(&conn).unwrap().is_some());
        assert!(st::load_one_time_prekey(&conn, first_id).is_err(), "lote antigo apagado");
        assert_eq!(st::count_one_time_prekeys(&conn).unwrap(), OPK_BATCH);
    }
}
