//! Prova que a identidade e as sessoes sobrevivem a fechar a ligacao a base
//! de dados e reabri-la - o cenario real de "fechar a app e voltar a abrir
//! mais tarde". Tambem prova que uma passphrase errada nao consegue ler os
//! dados (a cifra do SQLCipher esta mesmo a proteger o conteudo).

use secure_messenger_core::primitives::{DhKeyPair, SigningKeyPair};
use secure_messenger_core::ratchet::RatchetState;
use secure_messenger_core::x3dh::{sign_pre_key, x3dh_initiate, x3dh_respond, PreKeyBundle};
use secure_messenger_storage::{load_identity, load_session, open_database, save_identity, save_session, StoredIdentity};

fn temp_db_path(nome: &str) -> String {
    format!("/tmp/secure_messenger_test_{}_{}.sqlite", nome, std::process::id())
}

#[test]
fn identidade_sobrevive_a_fechar_e_reabrir() {
    let db_path = temp_db_path("identidade");
    let _ = std::fs::remove_file(&db_path);

    let identidade_original = StoredIdentity {
        identity: DhKeyPair::generate(),
        identity_signing: SigningKeyPair::generate(),
        signed_pre_key: DhKeyPair::generate(),
        one_time_pre_key: DhKeyPair::generate(),
    };
    let identity_public_original = identidade_original.identity.public;

    // ---------- "Primeiro arranque": criar e guardar a identidade ----------
    {
        let conn = open_database(&db_path, "palavra-passe-de-teste-forte").unwrap();
        save_identity(&conn, &identidade_original).unwrap();
    } // conn sai de escopo aqui - simula fechar a app

    // ---------- "Reabrir a app": nova ligacao, mesma passphrase ----------
    {
        let conn = open_database(&db_path, "palavra-passe-de-teste-forte").unwrap();
        let identidade_recarregada = load_identity(&conn).unwrap();
        assert_eq!(
            identidade_recarregada.identity.public.as_bytes(),
            identity_public_original.as_bytes(),
            "a identidade recarregada deve ser exatamente a mesma"
        );
    }

    std::fs::remove_file(&db_path).ok();
}

#[test]
fn passphrase_errada_nao_consegue_ler_os_dados() {
    let db_path = temp_db_path("passphrase_errada");
    let _ = std::fs::remove_file(&db_path);

    let identidade = StoredIdentity {
        identity: DhKeyPair::generate(),
        identity_signing: SigningKeyPair::generate(),
        signed_pre_key: DhKeyPair::generate(),
        one_time_pre_key: DhKeyPair::generate(),
    };

    {
        let conn = open_database(&db_path, "passphrase-correta").unwrap();
        save_identity(&conn, &identidade).unwrap();
    }

    // Reabre com a passphrase ERRADA - a leitura deve falhar (o SQLCipher
    // nao consegue decifrar as paginas da base de dados com a chave errada).
    // Isto pode falhar logo na abertura (ao tentar validar o schema) ou so
    // na leitura explicita - ambos os casos provam que a passphrase errada
    // nao da acesso aos dados, que e o que este teste quer confirmar.
    let resultado_final = open_database(&db_path, "passphrase-completamente-errada")
        .and_then(|conn| load_identity(&conn));

    assert!(
        resultado_final.is_err(),
        "aceder aos dados com a passphrase errada deveria falhar nalgum ponto, nao devolver dados legiveis"
    );

    std::fs::remove_file(&db_path).ok();
}

#[test]
fn sessao_de_ratchet_sobrevive_a_fechar_e_reabrir() {
    let db_path = temp_db_path("sessao");
    let _ = std::fs::remove_file(&db_path);

    // Estabelece uma sessao E2EE completa (igual aos testes de core/).
    let alice_identity = DhKeyPair::generate();
    let bob_identity = DhKeyPair::generate();
    let bob_identity_signing = SigningKeyPair::generate();
    let bob_signed_pre_key = DhKeyPair::generate();
    let bob_one_time_pre_key = DhKeyPair::generate();

    let signature = sign_pre_key(&bob_identity_signing, &bob_signed_pre_key.public);
    let bob_bundle = PreKeyBundle {
        identity_key: bob_identity.public,
        identity_signing_key: bob_identity_signing.verifying_key,
        signed_pre_key: bob_signed_pre_key.public,
        signed_pre_key_signature: signature,
        one_time_pre_key: Some(bob_one_time_pre_key.public),
    };

    let init_result = x3dh_initiate(&alice_identity, &bob_bundle).unwrap();
    let bob_shared_secret = x3dh_respond(
        &bob_identity,
        &bob_signed_pre_key,
        Some(&bob_one_time_pre_key),
        &alice_identity.public,
        &init_result.ephemeral_public,
    );

    let mut alice_ratchet =
        RatchetState::init_as_initiator(init_result.shared_secret, bob_signed_pre_key.public);

    // Alice envia uma mensagem e GUARDA o estado da sua sessao logo a seguir.
    let msg1 = alice_ratchet.encrypt(b"mensagem antes de fechar a app").unwrap();
    {
        let conn = open_database(&db_path, "outra-passphrase-forte").unwrap();
        save_session(&conn, "bob", &alice_ratchet).unwrap();
    }
    drop(alice_ratchet); // simula fechar a app - o estado em memoria desaparece

    // "Reabrir a app": carregar a sessao da base de dados e continuar a conversa.
    let mut alice_ratchet_recarregada = {
        let conn = open_database(&db_path, "outra-passphrase-forte").unwrap();
        load_session(&conn, "bob").unwrap().expect("sessao deveria existir")
    };

    let msg2 = alice_ratchet_recarregada.encrypt(b"mensagem depois de reabrir").unwrap();

    // Confirma do lado de Bob (que nunca fechou/reabriu, so para verificar
    // que a sessao recarregada ainda produz mensagens validas e consistentes).
    let mut bob_ratchet = RatchetState::init_as_responder(bob_shared_secret, bob_signed_pre_key);
    assert_eq!(bob_ratchet.decrypt(&msg1).unwrap(), b"mensagem antes de fechar a app");
    assert_eq!(bob_ratchet.decrypt(&msg2).unwrap(), b"mensagem depois de reabrir");

    std::fs::remove_file(&db_path).ok();
}
