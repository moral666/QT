use qt_core::primitives::DhKeyPair;
use qt_core::sealed_sender::{seal_sender_identity, unseal_sender_identity};

#[test]
fn destinatario_correto_descobre_o_remetente() {
    let alice_identity = DhKeyPair::generate(); // remetente
    let bob_identity = DhKeyPair::generate(); // destinatario

    let envelope = seal_sender_identity(&alice_identity.public, &bob_identity.public).unwrap();

    let remetente_revelado =
        unseal_sender_identity(&bob_identity, &envelope).expect("Bob deveria conseguir abrir");

    assert_eq!(
        remetente_revelado.as_bytes(),
        alice_identity.public.as_bytes(),
        "a identidade revelada deve ser exatamente a de Alice"
    );
}

#[test]
fn terceiro_nao_consegue_abrir_o_envelope() {
    // Simula o SERVIDOR (ou qualquer outra parte) a tentar ler o envelope
    // sem ser o destinatario pretendido - isto e o que o sealed sender
    // esta a proteger.
    let alice_identity = DhKeyPair::generate();
    let bob_identity = DhKeyPair::generate();
    let atacante_identity = DhKeyPair::generate();

    let envelope = seal_sender_identity(&alice_identity.public, &bob_identity.public).unwrap();

    let resultado = unseal_sender_identity(&atacante_identity, &envelope);

    // A "abertura" pode devolver Ok com um resultado errado (nao ha
    // integridade de identidade contra chave errada dentro do AEAD em si -
    // o AEAD deteta adulteracao, nao "chave errada" per se) OU falhar,
    // dependendo da matematica - o que importa e que NUNCA revele a
    // identidade correta de Alice.
    match resultado {
        Ok(chave_errada) => assert_ne!(
            chave_errada.as_bytes(),
            alice_identity.public.as_bytes(),
            "um atacante NUNCA deveria conseguir reconstruir a identidade correta"
        ),
        Err(_) => {} // falhar tambem e um resultado aceitavel e esperado
    }
}

#[test]
fn envelope_adulterado_falha_a_decifrar() {
    let alice_identity = DhKeyPair::generate();
    let bob_identity = DhKeyPair::generate();

    let mut envelope = seal_sender_identity(&alice_identity.public, &bob_identity.public).unwrap();
    let last = envelope.len() - 1;
    envelope[last] ^= 0xFF; // adultera o ultimo byte do ciphertext

    let resultado = unseal_sender_identity(&bob_identity, &envelope);
    assert!(resultado.is_err(), "envelope adulterado deveria falhar a decifragem (deteção do AEAD)");
}

#[test]
fn caixa_selada_ligada_a_um_binding_nao_pode_ser_reutilizada() {
    use qt_core::sealed_sender::{seal_sender_identity_bound, unseal_sender_identity_bound};
    let alice = DhKeyPair::generate();
    let bob = DhKeyPair::generate();
    let env = seal_sender_identity_bound(&alice.public, &bob.public, b"envelope-1").unwrap();
    assert_eq!(unseal_sender_identity_bound(&bob, &env, b"envelope-1").unwrap().as_bytes(), alice.public.as_bytes());
    assert!(unseal_sender_identity_bound(&bob, &env, b"envelope-2").is_err());
    assert!(unseal_sender_identity(&bob, &env).is_err());
}
