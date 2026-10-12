//! Testes diretos da API que o Android vai usar (`QtClient`), sem Kotlin.

use qt_ffi::*;
use std::sync::Arc;

fn tmp(n: &str) -> String {
    // Contador global: cada chamada devolve um caminho UNICO (os testes correm
    // em paralelo no mesmo processo e varios usam o mesmo nome, ex. "alice").
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_ffi_{}_{}_{}.sqlite", n, std::process::id(), id);
    let _ = std::fs::remove_file(&p);
    p
}
const KA: [u8; 32] = [0x11; 32];
const KB: [u8; 32] = [0x22; 32];

struct Peer {
    path: String,
    key: [u8; 32],
    c: Arc<QtClient>,
    id: PublicIdentity,
}
fn peer(name: &str, key: [u8; 32]) -> Peer {
    let path = tmp(name);
    let c = QtClient::open(path.clone(), key.to_vec()).unwrap();
    let id = c.create_identity().unwrap();
    Peer { path, key, c, id }
}
impl Peer {
    fn restart(&mut self) {
        self.c = QtClient::open(self.path.clone(), self.key.to_vec()).unwrap();
    }
    fn peer_bundle(&self, n: usize) -> PeerBundle {
        let b = self.c.generate_prekey_bundle(5).unwrap();
        PeerBundle {
            identity_public: b.identity_public,
            signing_public: b.signing_public,
            signed_pre_key_public: b.signed_pre_key_public,
            signed_pre_key_signature: b.signed_pre_key_signature,
            pq_pre_key_id: b.pq_pre_key_id,
            pq_pre_key_public: b.pq_pre_key_public,
            one_time_pre_key: b.one_time_pre_keys.get(n).cloned(),
        }
    }
}
fn cid(p: &Peer) -> String {
    p.id.identity_public.iter().map(|b| format!("{:02x}", b)).collect()
}
fn setup() -> (Peer, Peer, String, String) {
    let a = peer("alice", KA);
    let b = peer("bob", KB);
    let b_for_alice = a.c.start_session(b.id.public_id.clone(), b.peer_bundle(0)).unwrap();
    (a, b, b_for_alice.clone(), b_for_alice)
}

#[test]
fn conversa_completa_nos_dois_sentidos() {
    let (a, b, bob_cid, _) = setup();
    let alice_cid = cid(&a);
    assert_eq!(bob_cid, cid(&b));
    let e1 = a.c.encrypt(bob_cid.clone(), b"ola bob".to_vec()).unwrap();
    assert_eq!(b.c.decrypt(alice_cid.clone(), e1).unwrap(), b"ola bob");
    assert!(b.c.has_session(alice_cid.clone()).unwrap());
    let e2 = b.c.encrypt(alice_cid.clone(), b"ola alice".to_vec()).unwrap();
    assert_eq!(a.c.decrypt(bob_cid.clone(), e2).unwrap(), b"ola alice");
    for i in 0..20u8 {
        let e = a.c.encrypt(bob_cid.clone(), vec![i; 10]).unwrap();
        assert_eq!(b.c.decrypt(alice_cid.clone(), e).unwrap(), vec![i; 10]);
        let e = b.c.encrypt(alice_cid.clone(), vec![i; 7]).unwrap();
        assert_eq!(a.c.decrypt(bob_cid.clone(), e).unwrap(), vec![i; 7]);
    }
}

#[test]
fn conversa_continua_depois_de_reiniciar_ambos() {
    let (mut a, mut b, bob_cid, _) = setup();
    let alice_cid = cid(&a);
    let e = a.c.encrypt(bob_cid.clone(), b"antes".to_vec()).unwrap();
    assert_eq!(b.c.decrypt(alice_cid.clone(), e).unwrap(), b"antes");
    a.restart();
    b.restart();
    let e = b.c.encrypt(alice_cid.clone(), b"depois do restart".to_vec()).unwrap();
    assert_eq!(a.c.decrypt(bob_cid.clone(), e).unwrap(), b"depois do restart");
    let e = a.c.encrypt(bob_cid.clone(), b"e de volta".to_vec()).unwrap();
    assert_eq!(b.c.decrypt(alice_cid, e).unwrap(), b"e de volta");
}

#[test]
fn replay_e_rejeitado_tambem_depois_de_reiniciar() {
    let (a, mut b, bob_cid, _) = setup();
    let alice_cid = cid(&a);
    let e = a.c.encrypt(bob_cid, b"uma vez".to_vec()).unwrap();
    assert_eq!(b.c.decrypt(alice_cid.clone(), e.clone()).unwrap(), b"uma vez");
    assert!(matches!(b.c.decrypt(alice_cid.clone(), e.clone()), Err(QtError::Replay)));
    b.restart();
    assert!(matches!(b.c.decrypt(alice_cid, e), Err(QtError::Replay)));
}

#[test]
fn ciphertext_alterado_falha_e_nao_estraga_o_estado() {
    let (a, b, bob_cid, _) = setup();
    let alice_cid = cid(&a);
    let e1 = a.c.encrypt(bob_cid.clone(), b"primeira".to_vec()).unwrap();
    assert_eq!(b.c.decrypt(alice_cid.clone(), e1).unwrap(), b"primeira");
    let e2 = a.c.encrypt(bob_cid.clone(), b"segunda".to_vec()).unwrap();
    let mut bad = e2.clone();
    let last = bad.len() - 1;
    bad[last] ^= 0x01;
    assert!(matches!(b.c.decrypt(alice_cid.clone(), bad), Err(QtError::Crypto)));
    // Alterar n no cabecalho do ratchet tambem falha (AAD).
    let mut bad_n = e2.clone();
    let n_off = bad_n.len() - (b"segunda".len() + 16) - 1;
    bad_n[n_off] ^= 0x01;
    assert!(b.c.decrypt(alice_cid.clone(), bad_n).is_err());
    // O original ainda decifra: o estado nao foi corrompido.
    assert_eq!(b.c.decrypt(alice_cid, e2).unwrap(), b"segunda");
}

#[test]
fn fora_de_ordem_e_aceite_uma_vez() {
    let (a, b, bob_cid, _) = setup();
    let alice_cid = cid(&a);
    let e1 = a.c.encrypt(bob_cid.clone(), b"1".to_vec()).unwrap();
    let e2 = a.c.encrypt(bob_cid.clone(), b"2".to_vec()).unwrap();
    let e3 = a.c.encrypt(bob_cid, b"3".to_vec()).unwrap();
    assert_eq!(b.c.decrypt(alice_cid.clone(), e3.clone()).unwrap(), b"3");
    assert_eq!(b.c.decrypt(alice_cid.clone(), e1.clone()).unwrap(), b"1");
    assert_eq!(b.c.decrypt(alice_cid.clone(), e2).unwrap(), b"2");
    assert!(matches!(b.c.decrypt(alice_cid.clone(), e1), Err(QtError::Replay)));
    assert!(matches!(b.c.decrypt(alice_cid, e3), Err(QtError::Replay)));
}

#[test]
fn sessao_inexistente_e_lixo_nao_fazem_panic() {
    let b = peer("bob_lixo", KB);
    let fake_contact: String = "ab".repeat(32);
    assert!(matches!(b.c.encrypt(fake_contact.clone(), b"x".to_vec()), Err(QtError::UnknownSession)));
    // envelope normal (sem cabecalho) sem sessao
    let mut env = vec![1u8, 0u8];
    env.extend_from_slice(&[7u8; 60]);
    assert!(matches!(b.c.decrypt(fake_contact.clone(), env), Err(QtError::UnknownSession)));
    for junk in [vec![], vec![1u8], vec![9u8; 5], vec![1, 1, 0, 0], vec![1, 7, 1, 2, 3], vec![0xFF; 3000]] {
        assert!(b.c.decrypt(fake_contact.clone(), junk.clone()).is_err());
        assert!(b.c.decrypt_sealed(junk).is_err());
    }
    assert!(b.c.decrypt("nao-hex".into(), vec![1, 0]).is_err());
    assert!(b.c.decrypt("abcd".into(), vec![1, 0]).is_err());
}

#[test]
fn prekey_reutilizada_e_rejeitada() {
    let a = peer("a_reuse", KA);
    let c = peer("c_reuse", [0x33; 32]);
    let b = peer("b_reuse", KB);
    let bundle_a = b.peer_bundle(0);
    let bundle_c = PeerBundle { ..clone_bundle(&bundle_a) }; // mesma OPK id
    a.c.start_session(b.id.public_id.clone(), bundle_a).unwrap();
    c.c.start_session(b.id.public_id.clone(), bundle_c).unwrap();
    let ea = a.c.encrypt(cid(&b), b"da alice".to_vec()).unwrap();
    let ec = c.c.encrypt(cid(&b), b"da carol".to_vec()).unwrap();
    assert_eq!(b.c.decrypt(cid(&a), ea).unwrap(), b"da alice");
    // A OPK ja foi consumida pela Alice.
    assert!(matches!(b.c.decrypt(cid(&c), ec), Err(QtError::PrekeyUnavailable)));
}

#[test]
fn cabecalho_forjado_nao_gasta_a_prekey() {
    let a = peer("a_forge", KA);
    let m = peer("m_forge", [0x44; 32]);
    let b = peer("b_forge", KB);
    let bundle = b.peer_bundle(0);
    let opk_id = bundle.one_time_pre_key.as_ref().unwrap().id;
    let bundle2 = clone_bundle(&bundle);
    // Mallory (que conhece o bundle publico) forja um envelope com a mesma OPK.
    m.c.start_session(b.id.public_id.clone(), bundle).unwrap();
    let mut forged = m.c.encrypt(cid(&b), b"forjada".to_vec()).unwrap();
    let last = forged.len() - 1;
    forged[last] ^= 1;
    assert!(b.c.decrypt(cid(&m), forged).is_err());
    assert!(!b.c.has_session(cid(&m)).unwrap(), "nao fica sessao de um envelope invalido");
    // A OPK continua disponivel para a Alice legitima.
    a.c.start_session(b.id.public_id.clone(), bundle2).unwrap();
    let e = a.c.encrypt(cid(&b), b"legitima".to_vec()).unwrap();
    assert_eq!(b.c.decrypt(cid(&a), e).unwrap(), b"legitima");
    let _ = opk_id;
}

fn clone_bundle(b: &PeerBundle) -> PeerBundle {
    PeerBundle {
        identity_public: b.identity_public.clone(),
        signing_public: b.signing_public.clone(),
        signed_pre_key_public: b.signed_pre_key_public.clone(),
        signed_pre_key_signature: b.signed_pre_key_signature.clone(),
        pq_pre_key_id: b.pq_pre_key_id,
        pq_pre_key_public: b.pq_pre_key_public.clone(),
        one_time_pre_key: b.one_time_pre_key.clone(),
    }
}

#[test]
fn bundles_invalidos_sao_rejeitados() {
    let a = peer("a_bad", KA);
    let b = peer("b_bad", KB);
    let other = peer("o_bad", [0x55; 32]);
    // public_id nao corresponde ao bundle (relay malicioso entrega bundle de outro)
    assert!(matches!(
        a.c.start_session(b.id.public_id.clone(), other.peer_bundle(0)),
        Err(QtError::InvalidBundle)
    ));
    // assinatura adulterada
    let mut bad = b.peer_bundle(0);
    bad.signed_pre_key_signature[0] ^= 1;
    assert!(matches!(a.c.start_session(b.id.public_id.clone(), bad), Err(QtError::InvalidBundle)));
    // signed pre-key trocada
    let mut bad = b.peer_bundle(0);
    bad.signed_pre_key_public = other.peer_bundle(0).signed_pre_key_public;
    assert!(a.c.start_session(b.id.public_id.clone(), bad).is_err());
    // chave ML-KEM VALIDA mas de outra pessoa (relay malicioso a anular a protecao PQ)
    let mut bad = b.peer_bundle(0);
    bad.pq_pre_key_public = other.peer_bundle(0).pq_pre_key_public;
    assert!(matches!(a.c.start_session(b.id.public_id.clone(), bad), Err(QtError::InvalidBundle)));
    // chave PQ invalida (tamanho e conteudo)
    let mut bad = b.peer_bundle(0);
    bad.pq_pre_key_public = vec![0xFF; 1184];
    assert!(a.c.start_session(b.id.public_id.clone(), bad).is_err());
    let mut bad = b.peer_bundle(0);
    bad.pq_pre_key_public.truncate(100);
    assert!(a.c.start_session(b.id.public_id.clone(), bad).is_err());
    // public id mal formado
    for pid in ["", "qt1:", "qt1:zz", "xx:1234", "qt1:ab"] {
        assert!(a.c.start_session(pid.into(), b.peer_bundle(0)).is_err());
    }
    // sem OPK tambem funciona
    let mut nob = b.peer_bundle(0);
    nob.one_time_pre_key = None;
    let cidb = a.c.start_session(b.id.public_id.clone(), nob).unwrap();
    let e = a.c.encrypt(cidb, b"sem opk".to_vec()).unwrap();
    assert_eq!(b.c.decrypt(cid(&a), e).unwrap(), b"sem opk");
}

#[test]
fn sealed_sender_relay_nao_ve_plaintext_nem_remetente() {
    let (a, b, bob_cid, _) = setup();
    let alice_cid = cid(&a);
    let secret = b"conteudo muito secreto 12345";
    let blob = a.c.encrypt_sealed(bob_cid.clone(), secret.to_vec()).unwrap();
    // O que o relay ve: so `blob`.
    assert!(!blob.windows(secret.len()).any(|w| w == secret), "plaintext no blob");
    assert!(!blob.windows(32).any(|w| w == a.id.identity_public.as_slice()), "identidade do remetente visivel");
    assert!(!blob.windows(32).any(|w| w == a.id.signing_public.as_slice()));
    let r = b.c.decrypt_sealed(blob.clone()).unwrap();
    assert_eq!(r.plaintext, secret);
    assert_eq!(r.contact_id, alice_cid);
    assert!(matches!(b.c.decrypt_sealed(blob), Err(QtError::Replay)));
    // resposta tambem seladas
    let back = b.c.encrypt_sealed(alice_cid, b"resposta".to_vec()).unwrap();
    assert_eq!(a.c.decrypt_sealed(back).unwrap().plaintext, b"resposta");
    // um terceiro nao consegue abrir
    let eve = peer("eve", [0x66; 32]);
    let blob2 = a.c.encrypt_sealed(bob_cid, b"x".to_vec()).unwrap();
    assert!(eve.c.decrypt_sealed(blob2).is_err());
}

#[test]
fn envelope_normal_nao_contem_plaintext_nem_identidade() {
    let (a, _b, bob_cid, _) = setup();
    let secret = b"segredo-plaintext-xyz";
    let env = a.c.encrypt(bob_cid, secret.to_vec()).unwrap();
    assert!(!env.windows(secret.len()).any(|w| w == secret));
    assert!(!env.windows(32).any(|w| w == a.id.identity_public.as_slice()));
}

#[test]
fn identidade_ciclo_de_vida_e_chave_errada() {
    let mut a = peer("life", KA);
    assert!(a.c.has_identity().unwrap());
    assert!(matches!(a.c.create_identity(), Err(QtError::IdentityExists)));
    let pubid = a.c.export_public_identity().unwrap();
    assert_eq!(pubid.public_id, a.id.public_id);
    a.restart();
    assert_eq!(a.c.export_public_identity().unwrap().public_id, a.id.public_id, "mesma identidade apos reinicio");
    assert!(QtClient::open(a.path.clone(), vec![0u8; 32]).is_err(), "chave errada");
    assert!(QtClient::open(a.path.clone(), vec![0u8; 5]).is_err(), "chave com tamanho invalido");
    assert!(a.c.generate_prekey_bundle(100_000).is_err());
    a.c.delete_identity().unwrap();
    assert!(!a.c.has_identity().unwrap());
    assert!(matches!(a.c.export_public_identity(), Err(QtError::IdentityMissing)));
}

#[test]
fn ids_de_prekey_nunca_sao_reutilizados() {
    let a = peer("ids", KA);
    let b1 = a.c.generate_prekey_bundle(3).unwrap();
    let b2 = a.c.generate_prekey_bundle(3).unwrap();
    let mut ids: Vec<u32> = b1.one_time_pre_keys.iter().chain(b2.one_time_pre_keys.iter()).map(|k| k.id).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n);
    assert!(ids.iter().all(|i| *i != 0));
}
