//! PQXDH: persistencia (serializar -> "reiniciar" -> restaurar) e testes
//! negativos. Corre com `cargo test -p qt_core --features pq`.

use qt_core::pqxdh::*;
use qt_core::primitives::{DhKeyPair, SigningKeyPair};

struct Bob {
    identity: DhKeyPair,
    signing: SigningKeyPair,
    spk: DhKeyPair,
    pq: PqPreKeyPair,
    opk: DhKeyPair,
}

impl Bob {
    fn new() -> Self {
        Bob {
            identity: DhKeyPair::generate(),
            signing: SigningKeyPair::generate(),
            spk: DhKeyPair::generate(),
            pq: PqPreKeyPair::generate(),
            opk: DhKeyPair::generate(),
        }
    }
    fn bundle(&self, with_opk: bool) -> PqPreKeyBundle {
        PqPreKeyBundle {
            identity_key: self.identity.public,
            identity_signing_key: self.signing.verifying_key,
            signed_pre_key: self.spk.public,
            signed_pre_key_signature: sign_pre_key(&self.signing, &self.identity.public, &self.spk.public, &self.pq.public_key_bytes()),
            pq_pre_key: pq_public_key_from_bytes(&self.pq.public_key_bytes()).unwrap(),
            one_time_pre_key: if with_opk { Some(self.opk.public) } else { None },
        }
    }
}

#[test]
fn pq_prekey_sobrevive_a_serializacao_e_reinicio() {
    let alice = DhKeyPair::generate();
    let bob = Bob::new();
    let bundle = bob.bundle(true);

    // "processo reinicia": serializa, destroi o original, restaura.
    let blob = bob.pq.to_bytes();
    assert_eq!(blob.len(), PQ_PRIVATE_KEY_LEN);
    let pub_before = bob.pq.public_key_bytes();
    let Bob { identity, signing: _, spk, pq, opk } = bob;
    drop(pq);
    let restored = PqPreKeyPair::from_bytes(&blob).expect("restore");
    assert_eq!(restored.public_key_bytes(), pub_before, "mesma identidade PQ");

    let init = pqxdh_initiate(&alice, &bundle).unwrap();
    let bob_secret = pqxdh_respond(
        &identity, &spk, &restored, Some(&opk),
        &alice.public, &init.ephemeral_public, &init.pq_ciphertext,
    )
    .unwrap();
    assert_eq!(init.shared_secret, bob_secret);
}

#[test]
fn sem_one_time_prekey_tambem_coincide() {
    let alice = DhKeyPair::generate();
    let bob = Bob::new();
    let init = pqxdh_initiate(&alice, &bob.bundle(false)).unwrap();
    let s = pqxdh_respond(
        &bob.identity, &bob.spk, &bob.pq, None,
        &alice.public, &init.ephemeral_public, &init.pq_ciphertext,
    )
    .unwrap();
    assert_eq!(init.shared_secret, s);
}

#[test]
fn chave_publica_pq_invalida_e_rejeitada() {
    assert!(matches!(pq_public_key_from_bytes(&[]), Err(PqxdhError::InvalidPqPublicKey)));
    assert!(matches!(pq_public_key_from_bytes(&[0u8; 10]), Err(PqxdhError::InvalidPqPublicKey)));
    assert!(matches!(
        pq_public_key_from_bytes(&vec![0u8; PQ_PUBLIC_KEY_LEN + 1]),
        Err(PqxdhError::InvalidPqPublicKey)
    ));
    // Coeficientes 0xFFF >= q (3329): codificacao nao canonica.
    assert!(matches!(
        pq_public_key_from_bytes(&vec![0xFFu8; PQ_PUBLIC_KEY_LEN]),
        Err(PqxdhError::InvalidPqPublicKey)
    ));
}

#[test]
fn chave_privada_pq_corrompida_e_rejeitada() {
    let kp = PqPreKeyPair::generate();
    let blob = kp.to_bytes();
    assert!(PqPreKeyPair::from_bytes(&blob[..blob.len() - 1]).is_err());
    assert!(PqPreKeyPair::from_bytes(&[]).is_err());
    // Corromper um byte em varias zonas (dk_pke, ek embebida, H(ek)).
    for idx in [0usize, 700, 1200, 2000, 2340] {
        let mut bad = blob.clone();
        bad[idx] ^= 0x01;
        // Nao pode entrar em panico; ou falha, ou (se a zona nao for
        // verificada) tem de ser detetada pelo teste de coerencia.
        let r = std::panic::catch_unwind(|| PqPreKeyPair::from_bytes(&bad).is_ok());
        assert!(r.is_ok(), "panic ao desserializar estado corrompido");
    }
    let mut bad = blob.clone();
    bad[1300] ^= 0x01; // dentro da ek embebida
    assert!(PqPreKeyPair::from_bytes(&bad).is_err(), "ek embebida corrompida deve falhar");
    let mut bad = blob.clone();
    bad[10] ^= 0x01; // dentro de dk_pke: encaps/decaps deixam de coincidir
    assert!(PqPreKeyPair::from_bytes(&bad).is_err(), "dk_pke corrompida deve falhar");
}

#[test]
fn ciphertext_pq_com_tamanho_invalido_e_rejeitado() {
    assert!(matches!(pq_ciphertext_from_bytes(&[]), Err(PqxdhError::InvalidPqCiphertext)));
    assert!(matches!(
        pq_ciphertext_from_bytes(&vec![0u8; PQ_CIPHERTEXT_LEN - 1]),
        Err(PqxdhError::InvalidPqCiphertext)
    ));
    assert!(pq_ciphertext_from_bytes(&vec![0u8; PQ_CIPHERTEXT_LEN]).is_ok());
}

#[test]
fn ciphertext_pq_adulterado_nao_da_o_mesmo_segredo() {
    // ML-KEM usa "implicit rejection": nao ha erro, mas o segredo diverge.
    let alice = DhKeyPair::generate();
    let bob = Bob::new();
    let init = pqxdh_initiate(&alice, &bob.bundle(true)).unwrap();
    let mut ct_bytes = init.pq_ciphertext.as_slice().to_vec();
    ct_bytes[5] ^= 0x01;
    let tampered = pq_ciphertext_from_bytes(&ct_bytes).unwrap();
    let s = pqxdh_respond(
        &bob.identity, &bob.spk, &bob.pq, Some(&bob.opk),
        &alice.public, &init.ephemeral_public, &tampered,
    )
    .unwrap();
    assert_ne!(init.shared_secret, s);
}

#[test]
fn assinatura_invalida_da_signed_prekey_e_rejeitada() {
    let alice = DhKeyPair::generate();
    let bob = Bob::new();
    let mut bundle = bob.bundle(true);
    // Servidor malicioso troca a signed pre-key por outra.
    bundle.signed_pre_key = DhKeyPair::generate().public;
    assert!(matches!(
        pqxdh_initiate(&alice, &bundle),
        Err(PqxdhError::Crypto(qt_core::CryptoError::SignatureInvalid))
    ));
}

#[test]
fn segredo_depende_do_componente_pos_quantico() {
    // Mesmo material classico, mas Bob responde com OUTRA chave ML-KEM:
    // o segredo tem de divergir (prova que o KEM entra no HKDF).
    let alice = DhKeyPair::generate();
    let bob = Bob::new();
    let init = pqxdh_initiate(&alice, &bob.bundle(true)).unwrap();
    let outro_pq = PqPreKeyPair::generate();
    let s = pqxdh_respond(
        &bob.identity, &bob.spk, &outro_pq, Some(&bob.opk),
        &alice.public, &init.ephemeral_public, &init.pq_ciphertext,
    )
    .unwrap();
    assert_ne!(init.shared_secret, s);
}

#[test]
fn segredo_depende_do_componente_classico() {
    // Mesma chave ML-KEM, mas one-time prekey errada: tem de divergir.
    let alice = DhKeyPair::generate();
    let bob = Bob::new();
    let init = pqxdh_initiate(&alice, &bob.bundle(true)).unwrap();
    let opk_errada = DhKeyPair::generate();
    let s = pqxdh_respond(
        &bob.identity, &bob.spk, &bob.pq, Some(&opk_errada),
        &alice.public, &init.ephemeral_public, &init.pq_ciphertext,
    )
    .unwrap();
    assert_ne!(init.shared_secret, s);
    // E omitir a OPK (downgrade) tambem diverge.
    let s2 = pqxdh_respond(
        &bob.identity, &bob.spk, &bob.pq, None,
        &alice.public, &init.ephemeral_public, &init.pq_ciphertext,
    )
    .unwrap();
    assert_ne!(init.shared_secret, s2);
}

#[test]
fn dois_handshakes_dao_segredos_distintos() {
    let alice = DhKeyPair::generate();
    let bob = Bob::new();
    let a = pqxdh_initiate(&alice, &bob.bundle(true)).unwrap();
    let b = pqxdh_initiate(&alice, &bob.bundle(true)).unwrap();
    assert_ne!(a.shared_secret, b.shared_secret);
    assert_ne!(a.pq_ciphertext.as_slice(), b.pq_ciphertext.as_slice());
}

#[test]
fn relay_malicioso_nao_pode_substituir_a_chave_mlkem() {
    // O atacante troca a chave ML-KEM do bundle pela sua (que conhece), deixando
    // tudo o resto (incl. a assinatura) intacto. A assinatura cobre a chave
    // ML-KEM, por isso tem de ser REJEITADO (senao a protecao pos-quantica
    // seria anulada sem aviso).
    let alice = DhKeyPair::generate();
    let bob = Bob::new();
    let mut bundle = bob.bundle(true);
    let atacante_pq = PqPreKeyPair::generate();
    bundle.pq_pre_key = pq_public_key_from_bytes(&atacante_pq.public_key_bytes()).unwrap();
    assert!(matches!(
        pqxdh_initiate(&alice, &bundle),
        Err(PqxdhError::Crypto(qt_core::CryptoError::SignatureInvalid))
    ));
}

#[test]
fn assinatura_x3dh_antiga_nao_serve_para_o_bundle_pqxdh() {
    // Separacao de dominio: uma assinatura so da signed pre-key (formato X3DH)
    // nao e aceite como assinatura de um bundle PQXDH.
    let alice = DhKeyPair::generate();
    let bob = Bob::new();
    let mut bundle = bob.bundle(true);
    bundle.signed_pre_key_signature = qt_core::x3dh::sign_pre_key(&bob.signing, &bob.spk.public);
    assert!(pqxdh_initiate(&alice, &bundle).is_err());
}
