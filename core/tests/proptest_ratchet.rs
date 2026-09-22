//! Testes baseados em propriedades ("property-based testing") do Double
//! Ratchet: em vez de escrever casos fixos, gera CENTENAS de sequencias
//! aleatorias de mensagens (tamanhos variados, ordens de entrega
//! diferentes, mensagens perdidas), e verifica que uma propriedade
//! continua verdadeira em todas elas - "toda a mensagem entregue decifra
//! para o texto original exato, seja qual for a ordem".
//!
//! Isto e diferente do fuzzing em fuzz_lite.rs (que testa bytes
//! ALEATORIOS/invalidos contra parsers) - aqui os dados sao sempre
//! criptograficamente validos, o que varia e a ORDEM e os PADROES de
//! entrega, que e precisamente a superficie onde bugs de estado (Double
//! Ratchet) costumam esconder-se.

use proptest::prelude::*;
use qt_core::primitives::{DhKeyPair, SigningKeyPair};
use qt_core::ratchet::{EncryptedMessage, RatchetState};
use qt_core::x3dh::{sign_pre_key, x3dh_initiate, x3dh_respond, PreKeyBundle};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// Estabelece uma sessao Alice<->Bob completa (X3DH + inicializacao do
/// ratchet dos dois lados), pronta para trocar mensagens.
fn setup_session() -> (RatchetState, RatchetState) {
    let alice_identity = DhKeyPair::generate();
    let bob_identity = DhKeyPair::generate();
    let bob_identity_signing = SigningKeyPair::generate();
    let bob_signed_pre_key = DhKeyPair::generate();

    let signature = sign_pre_key(&bob_identity_signing, &bob_signed_pre_key.public);
    let bob_bundle = PreKeyBundle {
        identity_key: bob_identity.public,
        identity_signing_key: bob_identity_signing.verifying_key,
        signed_pre_key: bob_signed_pre_key.public,
        signed_pre_key_signature: signature,
        one_time_pre_key: None,
    };

    let init_result = x3dh_initiate(&alice_identity, &bob_bundle).unwrap();
    let bob_shared_secret = x3dh_respond(
        &bob_identity,
        &bob_signed_pre_key,
        None,
        &alice_identity.public,
        &init_result.ephemeral_public,
    );

    let alice_state = RatchetState::init_as_initiator(init_result.shared_secret, bob_signed_pre_key.public);
    let bob_state = RatchetState::init_as_responder(bob_shared_secret, bob_signed_pre_key);
    (alice_state, bob_state)
}

fn clone_message(msg: &EncryptedMessage) -> EncryptedMessage {
    EncryptedMessage { dh_public: msg.dh_public, n: msg.n, ciphertext: msg.ciphertext.clone() }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// A propriedade principal: Alice cifra N mensagens em sequencia:
    /// algumas sao "perdidas" (nunca entregues), as restantes chegam a
    /// Bob numa ordem ALEATORIA. Cada uma das que chegam deve decifrar
    /// exatamente para o texto original - nunca para outra coisa, e
    /// nunca deve falhar so por ter chegado fora de ordem.
    #[test]
    fn mensagens_reordenadas_e_perdidas_decifram_para_o_texto_original(
        plaintexts in prop::collection::vec(prop::collection::vec(any::<u8>(), 1..64), 1..40),
        shuffle_seed in any::<u64>(),
        drop_mask in prop::collection::vec(any::<bool>(), 1..40),
    ) {
        let (mut alice, mut bob) = setup_session();

        let encrypted: Vec<(Vec<u8>, EncryptedMessage)> = plaintexts
            .iter()
            .map(|pt| (pt.clone(), alice.encrypt(pt).unwrap()))
            .collect();

        let n = encrypted.len();
        let mut entregues: Vec<usize> = (0..n)
            .filter(|&i| *drop_mask.get(i).unwrap_or(&true))
            .collect();

        if entregues.is_empty() {
            return Ok(());
        }

        // Baralha deterministicamente (seed vem do proptest, reproduzivel
        // se algum dia falhar).
        let mut rng = StdRng::seed_from_u64(shuffle_seed);
        for i in (1..entregues.len()).rev() {
            let j = rng.gen_range(0..=i);
            entregues.swap(i, j);
        }

        for &idx in &entregues {
            let (original_pt, msg) = &encrypted[idx];
            let decrypted = bob.decrypt(msg);
            prop_assert!(
                decrypted.is_ok(),
                "falhou a decifrar a mensagem {idx} (entregue fora de ordem): {:?}",
                decrypted.err()
            );
            prop_assert_eq!(
                &decrypted.unwrap(),
                original_pt,
                "plaintext nao bate certo para a mensagem {}",
                idx
            );
        }
    }

    /// Alterna o sentido da conversa (Alice->Bob, Bob->Alice, repetido)
    /// muitas vezes seguidas - testa que o DH ratchet step (a parte mais
    /// delicada do protocolo) continua consistente ao longo de centenas
    /// de trocas de sentido.
    #[test]
    fn conversa_alternada_muitas_vezes_mantem_se_consistente(
        mensagens in prop::collection::vec(prop::collection::vec(any::<u8>(), 1..32), 10..100),
    ) {
        let (mut alice, mut bob) = setup_session();

        for (i, texto) in mensagens.iter().enumerate() {
            if i % 2 == 0 {
                let msg = alice.encrypt(texto).unwrap();
                let decrypted = bob.decrypt(&msg);
                prop_assert!(decrypted.is_ok(), "Bob falhou a decifrar a troca {i}");
                prop_assert_eq!(&decrypted.unwrap(), texto);
            } else {
                let msg = bob.encrypt(texto).unwrap();
                let decrypted = alice.decrypt(&msg);
                prop_assert!(decrypted.is_ok(), "Alice falhou a decifrar a troca {i}");
                prop_assert_eq!(&decrypted.unwrap(), texto);
            }
        }
    }
}

/// Nao e uma propriedade aleatoria - e um caso especifico e importante:
/// reenviar a MESMA mensagem cifrada uma segunda vez (replay) tem de
/// falhar. A chave de mensagem e destruida apos o primeiro uso.
#[test]
fn mensagem_duplicada_replay_falha_na_segunda_tentativa() {
    let (mut alice, mut bob) = setup_session();

    let msg = alice.encrypt(b"mensagem unica").unwrap();
    let copia_para_replay = clone_message(&msg);

    let primeira_tentativa = bob.decrypt(&msg);
    assert!(primeira_tentativa.is_ok(), "a primeira entrega deveria funcionar normalmente");

    let segunda_tentativa = bob.decrypt(&copia_para_replay);
    assert!(
        segunda_tentativa.is_err(),
        "reenviar a mesma mensagem (replay) deveria falhar - a chave ja foi consumida"
    );
}

/// Mesmo teste, mas para uma mensagem que chegou fora de ordem (skipped
/// key) - o replay tem de falhar tambem neste caminho, nao so no caminho
/// "normal" testado acima.
#[test]
fn mensagem_duplicada_de_uma_skipped_key_tambem_falha_no_replay() {
    let (mut alice, mut bob) = setup_session();

    let m1 = alice.encrypt(b"primeira").unwrap();
    let m2 = alice.encrypt(b"segunda").unwrap();
    let copia_m2 = clone_message(&m2);

    // Bob recebe m2 primeiro (m1 fica "skipped"), depois recebe a copia de
    // m2 outra vez - deve falhar, mesmo vindo do caminho de skipped keys.
    let primeira = bob.decrypt(&m2);
    assert!(primeira.is_ok());

    let replay = bob.decrypt(&copia_m2);
    assert!(replay.is_err(), "replay de uma mensagem que veio fora de ordem tambem deveria falhar");

    // m1, entregue depois, ainda deve funcionar normalmente (nao foi afetada).
    let m1_decrypted = bob.decrypt(&m1);
    assert!(m1_decrypted.is_ok(), "m1 (a skipped key genuina) ainda deveria decifrar corretamente");
}
