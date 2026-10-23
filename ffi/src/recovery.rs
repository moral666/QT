//! Recuperacao da conta (desde a 0.9.19): 12 palavras abrem uma copia cifrada no relay com
//! a identidade, os contactos (nomes e IDs), os bloqueados e o meu badge. Os grupos e o
//! historico de mensagens nao vao na copia.
//!
//!   * ATIVAR (`recovery_enable`): palavras novas; a chave e o identificador da copia ficam
//!     na base de dados cifrada. A app mostra as palavras uma vez (e de novo nas Definicoes).
//!   * COPIA (`recovery_backup`): a app passa a lista de contactos (vive no Kotlin/Swift); o
//!     Rust junta o resto, cifra e devolve os pedidos `BackupPut` (partes de 40 KB). So muda
//!     quando o conteudo muda (`recovery_backup_sent` guarda o resumo do que ja foi).
//!   * RECUPERAR, num telemovel sem identidade: `recovery_prepare(palavras)` (Argon2, ~1 s),
//!     `relay_request_backup_get(parte)` ate `parts` (a ligacao autentica-se com uma chave
//!     temporaria) e `recovery_restore(partes)`: instala a mesma identidade e marca todos os
//!     contactos para recomecar a sessao (a app abre sessoes novas e manda o convite; o outro
//!     lado troca para a sessao nova sozinho).
//!
//! Conteudo (JSON, antes de cifrar): `{"v":1, "identity", "signing" (hex 32), "contacts":
//! [{"n" nome, "p" public_id}], "blocked": [contact_id], "badge": hex | null, "created_at_ms"}`.

use crate::client::{bad, hex, public_identity, relock, QtClient, QtError, PQ_PREKEY_ID};
use crate::history::{now_ms, KV_BLOCKED, KV_RESTART, KV_SEND_INTRO};
use qt_core::pqxdh::PqPreKeyPair;
use qt_core::primitives::{DhKeyPair, SigningKeyPair};
use qt_core::recovery as qr;
use qt_storage as st;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// Maior parte enviada ao relay (igual a `BACKUP_PART_MAX` do relay).
pub const PART_MAX: usize = 40 * 1024;
/// Contactos na copia, no maximo.
pub const MAX_CONTACTS: usize = 2000;
const NICK_MAX: usize = 200;

const KV_ENTROPY: &str = "recovery:entropy";
const KV_KEY: &str = "recovery:key";
const KV_LOOKUP: &str = "recovery:lookup";
const KV_DIGEST: &str = "recovery:digest";

/// Recuperacao a meio: as palavras ja conferidas e o que delas sai (so em memoria).
pub(crate) struct PendingRecovery {
    entropy: Zeroizing<[u8; 16]>,
    keys: qr::RecoveryKeys,
}

/// Um contacto guardado na app (nome que lhe dei e o QT Public ID).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BackupContact {
    pub nickname: String,
    pub public_id: String,
}

/// Pedidos `BackupPut` a enviar por ordem (vazio: nada mudou desde a ultima copia).
/// Depois de o relay responder `Ack` a todos, chamar `recovery_backup_sent(digest)`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct BackupUpload {
    pub requests: Vec<Vec<u8>>,
    pub digest: String,
}

/// Conta recuperada: a app guarda os contactos na sua lista.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RestoredAccount {
    pub public_id: String,
    pub contacts: Vec<BackupContact>,
}

/// A palavra existe na lista? (para avisar logo de uma palavra mal escrita).
#[uniffi::export]
pub fn recovery_is_word(word: String) -> bool {
    qr::is_word(&word)
}

fn derive(entropy: &[u8; 16]) -> Result<qr::RecoveryKeys, QtError> {
    #[cfg(test)]
    let r = qr::derive_fast_for_tests(entropy);
    #[cfg(not(test))]
    let r = qr::derive(entropy);
    r.map_err(|_| QtError::Crypto)
}

fn words_err(_: qr::RecoveryError) -> QtError {
    bad("palavras")
}

fn unhex32(s: &str) -> Option<Zeroizing<[u8; 32]>> {
    if s.len() != 64 {
        return None;
    }
    let mut out = Zeroizing::new([0u8; 32]);
    for i in 0..32 {
        out[i] = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

fn valid_public_id(p: &str) -> bool {
    p.len() == 132 && p.starts_with("qt1:") && p[4..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn json_bytes(v: serde_json::Value) -> Result<Vec<u8>, QtError> {
    serde_json::to_vec(&v).map_err(|_| QtError::Storage)
}

#[uniffi::export]
impl QtClient {
    /// A chave de recuperacao esta ativa neste telemovel?
    pub fn recovery_enabled(&self) -> Result<bool, QtError> {
        let c = self.lock()?;
        Ok(st::kv_get(&c, KV_LOOKUP)?.is_some())
    }

    /// Ativa (ou troca) a chave de recuperacao: devolve as 12 palavras novas. A copia das
    /// palavras antigas e apagada pelo relay quando chegar a primeira copia nova. Demora
    /// ~1 s (Argon2): chamar fora da thread principal.
    pub fn recovery_enable(&self) -> Result<Vec<String>, QtError> {
        let entropy = qr::new_entropy();
        let keys = derive(&entropy)?;
        let c = self.lock()?;
        if !st::identity_exists(&c)? {
            return Err(QtError::IdentityMissing);
        }
        let tx = c.unchecked_transaction()?;
        st::kv_set(&tx, KV_ENTROPY, entropy.as_ref())?;
        st::kv_set(&tx, KV_KEY, keys.key.as_ref())?;
        st::kv_set(&tx, KV_LOOKUP, keys.lookup.as_bytes())?;
        st::kv_delete(&tx, KV_DIGEST)?;
        tx.commit()?;
        Ok(qr::words_of(&entropy))
    }

    /// As palavras atuais (para as ver de novo nas Definicoes), se a chave estiver ativa.
    pub fn recovery_words(&self) -> Result<Option<Vec<String>>, QtError> {
        let c = self.lock()?;
        let Some(e) = st::kv_get(&c, KV_ENTROPY)? else { return Ok(None) };
        let e = Zeroizing::new(e);
        let arr: [u8; 16] = e.as_slice().try_into().map_err(|_| QtError::Storage)?;
        let arr = Zeroizing::new(arr);
        Ok(Some(qr::words_of(&arr)))
    }

    /// Esquece a chave neste telemovel. Mandar antes `relay_request_backup_delete` para o
    /// relay apagar a copia.
    pub fn recovery_disable(&self) -> Result<(), QtError> {
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        for k in [KV_ENTROPY, KV_KEY, KV_LOOKUP, KV_DIGEST] {
            st::kv_delete(&tx, k)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Pedido `BackupDelete` (o relay apaga a copia desta conta). Resposta `Ack`.
    pub fn relay_request_backup_delete(&self) -> Result<Vec<u8>, QtError> {
        json_bytes(serde_json::json!("BackupDelete"))
    }

    /// Prepara a copia com os contactos da app. `force`: refaz mesmo que nada tenha mudado
    /// (a app chama assim de tempos a tempos). Sem chave ativa: nada a enviar.
    pub fn recovery_backup(&self, contacts: Vec<BackupContact>, force: bool) -> Result<BackupUpload, QtError> {
        if contacts.len() > MAX_CONTACTS {
            return Err(bad("contactos"));
        }
        let c = self.lock()?;
        let (Some(key), Some(lookup)) = (st::kv_get(&c, KV_KEY)?, st::kv_get(&c, KV_LOOKUP)?) else {
            return Ok(BackupUpload { requests: vec![], digest: String::new() });
        };
        let key = Zeroizing::new(key);
        let key: Zeroizing<[u8; 32]> = Zeroizing::new(key.as_slice().try_into().map_err(|_| QtError::Storage)?);
        let lookup = String::from_utf8(lookup).map_err(|_| QtError::Storage)?;
        let ident = st::load_identity(&c)?;
        let mut stmt = c.prepare("SELECT k FROM kv WHERE k LIKE ?1 ORDER BY k")?;
        let blocked: Vec<String> = stmt
            .query_map([format!("{KV_BLOCKED}%")], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|k| k[KV_BLOCKED.len()..].to_string())
            .collect();
        let badge = crate::badges::my_cert(&c)?.map(|b| hex(&b));
        let list: Vec<serde_json::Value> = contacts
            .iter()
            .filter(|x| valid_public_id(&x.public_id))
            .map(|x| serde_json::json!({"n": x.nickname.chars().take(NICK_MAX).collect::<String>(), "p": x.public_id}))
            .collect();
        // Resumo do que vai na copia (sem a hora): se for igual ao ultimo enviado, nada a fazer.
        let mut body = serde_json::json!({
            "v": 1,
            "identity": hex(&ident.identity.private.to_bytes()),
            "signing": hex(&ident.identity_signing.to_bytes()),
            "contacts": list,
            "blocked": blocked,
            "badge": badge,
        });
        let digest = {
            let plain = Zeroizing::new(serde_json::to_vec(&body).map_err(|_| QtError::Storage)?);
            let mut h = Sha256::new();
            h.update(lookup.as_bytes());
            h.update(&*plain);
            hex(&h.finalize())
        };
        if !force && st::kv_get(&c, KV_DIGEST)?.as_deref() == Some(digest.as_bytes()) {
            return Ok(BackupUpload { requests: vec![], digest });
        }
        body["created_at_ms"] = serde_json::json!(now_ms());
        let plain = Zeroizing::new(serde_json::to_vec(&body).map_err(|_| QtError::Storage)?);
        drop(body);
        let sealed = qr::seal(&key, &plain).map_err(|_| bad("copia"))?;
        let generation = hex(&crate::history::new_id()?[..8]);
        let chunks: Vec<&[u8]> = sealed.chunks(PART_MAX).collect();
        let parts = chunks.len() as u32;
        use base64::Engine;
        let requests = chunks
            .iter()
            .enumerate()
            .map(|(i, d)| {
                json_bytes(serde_json::json!({"BackupPut": {
                    "lookup": lookup, "generation": generation, "part": i as u32, "parts": parts,
                    "data": base64::engine::general_purpose::STANDARD.encode(d),
                }}))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(BackupUpload { requests, digest })
    }

    /// O relay aceitou todas as partes desta copia.
    pub fn recovery_backup_sent(&self, digest: String) -> Result<(), QtError> {
        let c = self.lock()?;
        st::kv_set(&c, KV_DIGEST, digest.as_bytes())?;
        Ok(())
    }

    /// Telemovel novo: confere as palavras e deriva a chave da copia (Argon2, ~1 s; chamar
    /// fora da thread principal). Palavras erradas: `InvalidInput { field: "palavras" }`.
    pub fn recovery_prepare(&self, words: Vec<String>) -> Result<(), QtError> {
        let entropy = qr::entropy_of(&words).map_err(words_err)?;
        let keys = derive(&entropy)?;
        *relock(&self.pending_recovery) = Some(PendingRecovery { entropy, keys });
        Ok(())
    }

    /// Pedido `BackupGet` (depois de `recovery_prepare`). Resposta: `BackupPart` ou
    /// `BackupNotFound` (nao ha copia para estas palavras).
    pub fn relay_request_backup_get(&self, part: u32) -> Result<Vec<u8>, QtError> {
        let pending = relock(&self.pending_recovery);
        let p = pending.as_ref().ok_or_else(|| bad("palavras"))?;
        json_bytes(serde_json::json!({"BackupGet": {"lookup": p.keys.lookup, "part": part}}))
    }

    /// Abre a copia (as partes por ordem) e instala a conta: mesma identidade, bloqueados e
    /// badge, e as mesmas palavras (as copias seguintes continuam a funcionar). So num
    /// telemovel sem identidade. As sessoes recomecam sozinhas (`sessions_to_restart`).
    pub fn recovery_restore(&self, parts: Vec<Vec<u8>>) -> Result<RestoredAccount, QtError> {
        let pending = relock(&self.pending_recovery).take().ok_or_else(|| bad("palavras"))?;
        let keys = &pending.keys;
        let sealed = parts.concat();
        let plain = qr::open(&keys.key, &sealed).map_err(|_| bad("copia"))?;
        let v: serde_json::Value = serde_json::from_slice(&plain).map_err(|_| bad("copia"))?;
        let field = |k: &str| v[k].as_str().and_then(unhex32).ok_or_else(|| bad("copia"));
        let (id_priv, sign_priv) = (field("identity")?, field("signing")?);
        let contacts: Vec<BackupContact> = v["contacts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| {
                let p = x["p"].as_str()?;
                valid_public_id(p).then(|| BackupContact {
                    nickname: x["n"].as_str().unwrap_or("").chars().take(NICK_MAX).collect(),
                    public_id: p.to_string(),
                })
            })
            .take(MAX_CONTACTS)
            .collect();
        let blocked: Vec<String> = v["blocked"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())))
            .map(str::to_string)
            .collect();
        let badge = v["badge"].as_str().and_then(|h| {
            (h.len() % 2 == 0).then(|| (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).ok()).collect::<Option<Vec<u8>>>())?
        });

        let identity = {
            let private = x25519_dalek::StaticSecret::from(*id_priv);
            let public = x25519_dalek::PublicKey::from(&private);
            DhKeyPair { private, public }
        };
        let ident = st::StoredIdentity {
            identity,
            identity_signing: SigningKeyPair::from_bytes(&sign_priv),
            signed_pre_key: DhKeyPair::generate(),
            one_time_pre_key: DhKeyPair::generate(),
        };
        let me = public_identity(&ident);

        let c = self.lock()?;
        if st::identity_exists(&c)? {
            return Err(QtError::IdentityExists);
        }
        let tx = c.unchecked_transaction()?;
        st::save_identity(&tx, &ident)?;
        st::save_pq_prekey(&tx, PQ_PREKEY_ID, &PqPreKeyPair::generate())?;
        for b in &blocked {
            st::kv_set(&tx, &format!("{KV_BLOCKED}{b}"), &[1])?;
        }
        if let Some(cert) = badge {
            crate::badges::restore_mine(&tx, &cert)?;
        }
        // Cada contacto: sessao nova com convite (o outro lado adota-a ao receber).
        for x in &contacts {
            let cid = x.public_id[4..68].to_ascii_lowercase();
            if blocked.contains(&cid) || cid == hex(me.identity_public.as_slice()) {
                continue;
            }
            st::kv_set(&tx, &format!("{KV_RESTART}{cid}"), &[1])?;
            st::kv_set(&tx, &format!("{KV_SEND_INTRO}{cid}"), &[1])?;
        }
        // As mesmas palavras continuam a valer neste telemovel.
        st::kv_set(&tx, KV_ENTROPY, pending.entropy.as_ref())?;
        st::kv_set(&tx, KV_KEY, keys.key.as_ref())?;
        st::kv_set(&tx, KV_LOOKUP, keys.lookup.as_bytes())?;
        tx.commit()?;
        *relock(&self.ephemeral_signing) = None;
        Ok(RestoredAccount { public_id: me.public_id, contacts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::peer_bundle_from_relay_bytes;
    use std::sync::Arc;

    fn client(name: &str, key: u8, identity: bool) -> Arc<QtClient> {
        let p = format!("/tmp/qt_recovery_{name}_{}.sqlite", std::process::id());
        let _ = std::fs::remove_file(&p);
        let c = QtClient::open(p, vec![key; 32]).unwrap();
        if identity {
            c.create_identity().unwrap();
        }
        c
    }

    fn cid(c: &QtClient) -> String {
        hex(&c.export_public_identity().unwrap().identity_public)
    }

    fn deliver(from: &QtClient, to: &QtClient) {
        for item in from.outbox().unwrap() {
            let _ = to.receive_sealed(item.blob.clone());
            from.outbox_sent(item.outbox_id).unwrap();
        }
    }

    fn texts(c: &QtClient, contact: &str) -> Vec<String> {
        c.messages_page(contact.to_string(), None, 100).unwrap().into_iter().map(|m| m.text).collect()
    }

    /// O relay simulado: devolve as partes de cada `BackupPut`.
    fn parts_of(up: &BackupUpload) -> Vec<Vec<u8>> {
        use base64::Engine;
        up.requests
            .iter()
            .map(|r| {
                let v: serde_json::Value = serde_json::from_slice(r).unwrap();
                base64::engine::general_purpose::STANDARD.decode(v["BackupPut"]["data"].as_str().unwrap()).unwrap()
            })
            .collect()
    }

    #[test]
    fn recuperar_traz_a_identidade_os_contactos_e_os_bloqueados_e_a_conversa_continua() {
        let alice = client("alice", 0x11, true);
        let bob = client("bob", 0x22, true);
        let bob_pid = bob.export_public_identity().unwrap().public_id;
        let alice_pid = alice.export_public_identity().unwrap().public_id;
        let bundle = peer_bundle_from_relay_bytes(bob.publishable_bundle().unwrap()).unwrap();
        alice.start_session(bob_pid.clone(), bundle).unwrap();
        alice.send_text(cid(&bob), "ola".into()).unwrap();
        deliver(&alice, &bob);
        bob.send_text(cid(&alice), "ola alice".into()).unwrap();
        deliver(&bob, &alice);
        let carol = "cc".repeat(32);
        alice.block_contact(carol.clone()).unwrap();

        // Sem chave ativa: nada a enviar.
        let contacts = vec![BackupContact { nickname: "Bob".into(), public_id: bob_pid.clone() }];
        assert!(alice.recovery_backup(contacts.clone(), true).unwrap().requests.is_empty());

        let words = alice.recovery_enable().unwrap();
        assert_eq!(words.len(), 12);
        assert!(alice.recovery_enabled().unwrap());
        assert_eq!(alice.recovery_words().unwrap(), Some(words.clone()));
        let up = alice.recovery_backup(contacts.clone(), false).unwrap();
        assert!(!up.requests.is_empty());
        alice.recovery_backup_sent(up.digest.clone()).unwrap();
        // Nada mudou: nada a enviar (a nao ser que se force).
        assert!(alice.recovery_backup(contacts.clone(), false).unwrap().requests.is_empty());
        assert!(!alice.recovery_backup(contacts.clone(), true).unwrap().requests.is_empty());
        let parts = parts_of(&up);

        // Telemovel novo, sem identidade: liga-se ao relay com uma chave temporaria.
        let novo = client("novo", 0x33, false);
        assert!(novo.relay_auth_message(vec![7u8; 32]).is_ok());
        assert!(novo.relay_request_backup_get(0).is_err(), "sem palavras");
        let mut wrong = words.clone();
        wrong.swap(0, 1);
        if wrong != words {
            // Palavras trocadas: ou a verificacao falha, ou a copia nao abre.
            if novo.recovery_prepare(wrong).is_ok() {
                assert!(novo.recovery_restore(parts.clone()).is_err());
            }
        }
        novo.recovery_prepare(words.iter().map(|w| w.to_uppercase()).collect()).unwrap();
        let req: serde_json::Value = serde_json::from_slice(&novo.relay_request_backup_get(0).unwrap()).unwrap();
        assert_eq!(req["BackupGet"]["part"], 0);
        let restored = novo.recovery_restore(parts).unwrap();

        assert_eq!(restored.public_id, alice_pid);
        assert_eq!(restored.contacts, contacts);
        assert_eq!(novo.export_public_identity().unwrap().public_id, alice_pid);
        assert!(novo.is_blocked(carol).unwrap());
        assert_eq!(novo.sessions_to_restart().unwrap(), vec![cid(&bob)]);
        assert_eq!(novo.recovery_words().unwrap(), Some(words), "as mesmas palavras continuam");
        assert!(novo.recovery_restore(vec![]).is_err(), "so uma vez");

        // A app abre uma sessao nova com o Bob: vai o convite e o Bob troca de sessao.
        let bundle = peer_bundle_from_relay_bytes(bob.publishable_bundle().unwrap()).unwrap();
        novo.start_session(bob_pid, bundle).unwrap();
        assert!(novo.sessions_to_restart().unwrap().is_empty());
        deliver(&novo, &bob);
        bob.send_text(cid(&novo), "bem-vinda de volta".into()).unwrap();
        deliver(&bob, &novo);
        assert!(texts(&novo, &cid(&bob)).contains(&"bem-vinda de volta".to_string()));
        novo.send_text(cid(&bob), "obrigada".into()).unwrap();
        deliver(&novo, &bob);
        assert!(texts(&bob, &cid(&novo)).contains(&"obrigada".to_string()));
    }

    #[test]
    fn depois_de_recuperar_os_contactos_voltam_a_mandar_o_badge_e_a_foto() {
        use qt_core::badge::{issue, issuer_public, BADGE_TESTER};
        const SEED: [u8; 32] = [0x5e; 32];
        crate::badges::set_test_issuer(issuer_public(&SEED));
        let alice = client("alice3", 0x17, true);
        let bob = client("bob3", 0x27, true);
        let alice_pid = alice.export_public_identity().unwrap().public_id;
        let bob_pid = bob.export_public_identity().unwrap().public_id;
        let bundle = peer_bundle_from_relay_bytes(bob.publishable_bundle().unwrap()).unwrap();
        alice.start_session(bob_pid.clone(), bundle).unwrap();
        alice.send_text(cid(&bob), "ola".into()).unwrap();
        deliver(&alice, &bob);
        bob.send_text(cid(&alice), "ola alice".into()).unwrap();
        deliver(&bob, &alice);

        // O Bob tem badge e foto e ja os mandou a Alice.
        let nuno = client("emissor3", 0x37, true);
        let bundle = peer_bundle_from_relay_bytes(bob.publishable_bundle().unwrap()).unwrap();
        nuno.start_session(bob_pid.clone(), bundle).unwrap();
        let bob_id: [u8; 32] = bob.export_public_identity().unwrap().identity_public.try_into().unwrap();
        nuno.badge_deliver(cid(&bob), issue(&SEED, &bob_id, BADGE_TESTER, 1_000)).unwrap();
        deliver(&nuno, &bob);
        let mut foto = b"RIFF\0\0\0\0WEBP".to_vec();
        foto.extend_from_slice(&[7u8; 64]);
        bob.profile_photo_set(foto.clone(), vec![cid(&alice)]).unwrap();
        assert_eq!(bob.badge_sync(vec![cid(&alice)]).unwrap(), 1);
        deliver(&bob, &alice);
        assert_eq!(alice.badge_of(cid(&bob)).unwrap(), BADGE_TESTER);
        assert_eq!(alice.profile_photo_of(cid(&bob)).unwrap(), Some(foto.clone()));

        // A Alice reinstala e recupera a conta com as 12 palavras.
        let words = alice.recovery_enable().unwrap();
        let contacts = vec![BackupContact { nickname: "Bob".into(), public_id: bob_pid.clone() }];
        let parts = parts_of(&alice.recovery_backup(contacts, false).unwrap());
        let novo = client("novo3", 0x47, false);
        novo.recovery_prepare(words).unwrap();
        assert_eq!(novo.recovery_restore(parts).unwrap().public_id, alice_pid);
        assert_eq!(novo.badge_of(cid(&bob)).unwrap(), 0);
        let bundle = peer_bundle_from_relay_bytes(bob.publishable_bundle().unwrap()).unwrap();
        novo.start_session(bob_pid, bundle).unwrap();
        deliver(&novo, &bob);

        // O Bob ve a sessao nova e volta a mandar o badge e a foto (antes da 0.9.29: nada).
        assert_eq!(bob.badge_sync(vec![cid(&novo)]).unwrap(), 1);
        assert_eq!(bob.profile_photo_sync(vec![cid(&novo)]).unwrap(), 1);
        deliver(&bob, &novo);
        assert_eq!(novo.badge_of(cid(&bob)).unwrap(), BADGE_TESTER);
        assert_eq!(novo.profile_photo_of(cid(&bob)).unwrap(), Some(foto));
        // So uma vez.
        assert_eq!(bob.badge_sync(vec![cid(&novo)]).unwrap(), 0);
    }

    #[test]
    fn copia_errada_ou_com_identidade_ja_criada_e_recusada() {
        let alice = client("a2", 0x44, true);
        let words = alice.recovery_enable().unwrap();
        let up = alice.recovery_backup(vec![], false).unwrap();
        let mut parts = parts_of(&up);

        // Ja com identidade: nao substitui.
        let other = client("o2", 0x55, true);
        other.recovery_prepare(words.clone()).unwrap();
        assert!(matches!(other.recovery_restore(parts.clone()), Err(QtError::IdentityExists)));

        // Bytes alterados: recusado.
        let novo = client("n2", 0x66, false);
        novo.recovery_prepare(words.clone()).unwrap();
        parts[0][40] ^= 1;
        assert!(novo.recovery_restore(parts).is_err());
        assert!(!novo.has_identity().unwrap());

        // Palavra inventada.
        let mut bad_words = words;
        bad_words[2] = "sealerx".into();
        assert!(novo.recovery_prepare(bad_words).is_err());
        assert!(recovery_is_word("abandon".into()));
        assert!(!recovery_is_word("sealerx".into()));
    }
}
