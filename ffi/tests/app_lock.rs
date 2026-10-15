//! Bloqueio da app com codigo proprio: a chave do armazenamento so sai com o codigo certo.

use qt_ffi::applock::*;
use qt_ffi::QtError;

const KEY: [u8; 32] = [7u8; 32];

#[test]
fn codigo_certo_devolve_a_chave() {
    let blob = app_lock_wrap("482916".into(), KEY.to_vec()).unwrap();
    assert_eq!(blob.len(), 92);
    assert_eq!(app_lock_unwrap("482916".into(), blob).unwrap(), KEY.to_vec());
}

#[test]
fn codigo_errado_falha() {
    let blob = app_lock_wrap("482916".into(), KEY.to_vec()).unwrap();
    for wrong in ["482917", "000000", "4829160", "abcdef"] {
        assert!(matches!(app_lock_unwrap(wrong.into(), blob.clone()), Err(QtError::Crypto)), "{wrong}");
    }
}

#[test]
fn codigo_curto_ou_enorme_e_recusado() {
    assert!(matches!(app_lock_wrap("12345".into(), KEY.to_vec()), Err(QtError::InvalidInput { .. })));
    assert!(matches!(app_lock_wrap("1".repeat(65), KEY.to_vec()), Err(QtError::InvalidInput { .. })));
    assert_eq!(app_lock_min_code_len(), 6);
}

#[test]
fn chave_com_tamanho_errado_e_recusada() {
    assert!(app_lock_wrap("482916".into(), vec![1u8; 31]).is_err());
}

#[test]
fn cada_bloqueio_usa_sal_e_nonce_novos() {
    let a = app_lock_wrap("482916".into(), KEY.to_vec()).unwrap();
    let b = app_lock_wrap("482916".into(), KEY.to_vec()).unwrap();
    assert_ne!(a, b);
    assert_ne!(a[16..32], b[16..32], "sal");
}

#[test]
fn qualquer_byte_alterado_faz_falhar() {
    let blob = app_lock_wrap("482916".into(), KEY.to_vec()).unwrap();
    // Sal, nonce, cifra e tag (os parametros tem um teste proprio abaixo).
    for i in [16, 31, 32, 43, 44, 60, 75, 76, 91] {
        let mut b = blob.clone();
        b[i] ^= 0x01;
        assert!(app_lock_unwrap("482916".into(), b).is_err(), "byte {i}");
    }
}

#[test]
fn parametros_alterados_nao_passam_nem_gastam_recursos() {
    let blob = app_lock_wrap("482916".into(), KEY.to_vec()).unwrap();
    // m = 1 GiB: recusado antes de tentar reservar memoria.
    let mut b = blob.clone();
    b[4..8].copy_from_slice(&(1024u32 * 1024).to_le_bytes());
    assert!(matches!(app_lock_unwrap("482916".into(), b), Err(QtError::InvalidInput { .. })));
    // t = 0 e p = 0: recusados.
    let mut b = blob.clone();
    b[8..12].copy_from_slice(&0u32.to_le_bytes());
    assert!(app_lock_unwrap("482916".into(), b).is_err());
    // Parametros validos mas diferentes dos usados: o cabecalho e autenticado, falha.
    let mut b = blob.clone();
    b[8..12].copy_from_slice(&2u32.to_le_bytes());
    assert!(matches!(app_lock_unwrap("482916".into(), b), Err(QtError::Crypto)));
}

#[test]
fn formato_desconhecido_e_recusado() {
    let mut blob = app_lock_wrap("482916".into(), KEY.to_vec()).unwrap();
    blob[0] = b'X';
    assert!(app_lock_unwrap("482916".into(), blob).is_err());
    assert!(app_lock_unwrap("482916".into(), vec![0u8; 10]).is_err());
}

#[test]
fn derivacao_demora_o_suficiente() {
    // Cada tentativa de codigo tem de custar tempo real (Argon2id, 64 MiB, t=3).
    let blob = app_lock_wrap("482916".into(), KEY.to_vec()).unwrap();
    let t = std::time::Instant::now();
    let _ = app_lock_unwrap("000000".into(), blob);
    let ms = t.elapsed().as_millis();
    println!("uma tentativa de codigo: {ms} ms");
    assert!(ms >= 20, "{ms} ms");
}
