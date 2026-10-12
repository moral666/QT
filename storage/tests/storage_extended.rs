//! Storage: chave binaria (Keystore), secure_delete, prekeys de uso unico,
//! PQ, Noise, metadados, dedup de envelopes e casos de corrupcao.

use qt_core::primitives::DhKeyPair;
use qt_storage::*;

fn tmp(n: &str) -> String {
    // Contador global: cada chamada devolve um caminho UNICO (os testes correm
    // em paralelo no mesmo processo e varios usam o mesmo nome, ex. "alice").
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = format!("/tmp/qt_ext_{}_{}_{}.sqlite", n, std::process::id(), id);
    let _ = std::fs::remove_file(&p);
    p
}

const KEY: [u8; 32] = [7u8; 32];

#[test]
fn chave_binaria_abre_reabre_e_chave_errada_falha() {
    let p = tmp("rawkey");
    {
        let c = open_database_with_key(&p, &KEY).unwrap();
        save_noise_static(&c, &[9u8; 32]).unwrap();
    }
    {
        let c = open_database_with_key(&p, &KEY).unwrap();
        assert_eq!(load_noise_static(&c).unwrap().unwrap(), vec![9u8; 32]);
    }
    let r = open_database_with_key(&p, &[8u8; 32]);
    assert!(matches!(r, Err(StorageError::CannotOpen)), "chave errada tem de falhar");
    assert!(matches!(open_database_with_key(&p, &[1u8; 31]), Err(StorageError::InvalidStorageKey)));
    assert!(matches!(open_database_with_key(&p, &[]), Err(StorageError::InvalidStorageKey)));
    std::fs::remove_file(&p).ok();
}

#[test]
fn erro_nunca_contem_a_chave() {
    let p = tmp("errkey");
    drop(open_database_with_key(&p, &KEY).unwrap());
    let e = open_database_with_key(&p, &[0xABu8; 32]).err().unwrap();
    let msg = format!("{e} {e:?}");
    assert!(!msg.to_lowercase().contains("abababab"));
    assert!(!msg.contains("x'"));
    std::fs::remove_file(&p).ok();
}

#[test]
fn ficheiro_em_disco_nao_tem_cabecalho_sqlite_nem_segredos() {
    let p = tmp("ondisk");
    let priv_marker = [0x5Au8; 32];
    {
        let c = open_database_with_key(&p, &KEY).unwrap();
        save_noise_static(&c, &priv_marker).unwrap();
    }
    let raw = std::fs::read(&p).unwrap();
    assert!(!raw.starts_with(b"SQLite format 3"), "ficheiro tem de estar cifrado");
    assert!(!raw.windows(32).any(|w| w == priv_marker), "chave privada visivel em disco");
    std::fs::remove_file(&p).ok();
}

#[test]
fn secure_delete_esta_ativo_na_configuracao_real() {
    let p = tmp("secdel");
    let c = open_database_with_key(&p, &KEY).unwrap();
    let v: i64 = c.query_row("PRAGMA secure_delete", [], |r| r.get(0)).unwrap();
    assert_eq!(v, 1);
    // Tambem na variante com passphrase.
    let p2 = tmp("secdel2");
    let c2 = open_database(&p2, "frase-de-teste").unwrap();
    let v2: i64 = c2.query_row("PRAGMA secure_delete", [], |r| r.get(0)).unwrap();
    assert_eq!(v2, 1);
    std::fs::remove_file(&p).ok();
    std::fs::remove_file(&p2).ok();
}

#[test]
fn nao_existe_tabela_de_mensagens() {
    let p = tmp("nomsg");
    let c = open_database_with_key(&p, &KEY).unwrap();
    let mut st = c.prepare("SELECT name FROM sqlite_master WHERE type='table'").unwrap();
    let nomes: Vec<String> = st.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    assert!(!nomes.iter().any(|n| n.contains("message") || n.contains("plaintext")), "{nomes:?}");
    std::fs::remove_file(&p).ok();
}

#[test]
fn one_time_prekey_so_pode_ser_consumida_uma_vez() {
    let p = tmp("opk");
    let mut c = open_database_with_key(&p, &KEY).unwrap();
    let k = DhKeyPair::generate();
    let pubk = k.public;
    save_one_time_prekey(&c, 42, &k).unwrap();
    assert_eq!(count_one_time_prekeys(&c).unwrap(), 1);
    let got = take_one_time_prekey(&mut c, 42).unwrap();
    assert_eq!(got.public.as_bytes(), pubk.as_bytes());
    assert_eq!(count_one_time_prekeys(&c).unwrap(), 0);
    // reutilizacao e prekey inexistente
    assert!(matches!(take_one_time_prekey(&mut c, 42), Err(StorageError::PrekeyNotFoundOrUsed)));
    assert!(matches!(take_one_time_prekey(&mut c, 999), Err(StorageError::PrekeyNotFoundOrUsed)));
    std::fs::remove_file(&p).ok();
}

#[test]
fn one_time_prekey_corrompida_da_erro_sem_panic() {
    let p = tmp("opkbad");
    let mut c = open_database_with_key(&p, &KEY).unwrap();
    c.execute("INSERT INTO one_time_prekeys (id, private) VALUES (1, x'0102')", []).unwrap();
    assert!(matches!(take_one_time_prekey(&mut c, 1), Err(StorageError::CorruptData)));
    std::fs::remove_file(&p).ok();
}

#[test]
fn metadados_de_sessao_e_dedup_de_envelopes() {
    let p = tmp("meta");
    {
        let c = open_database_with_key(&p, &KEY).unwrap();
        let m = SessionMeta {
            contact_id: "bob".into(),
            peer_identity_public: vec![3u8; 32],
            protocol: "pqxdh".into(),
            last_recv_seq: 5,
            send_seq: 9,
        };
        save_session_meta(&c, &m).unwrap();
        assert!(record_envelope(&c, "bob", b"env-1").unwrap(), "primeiro: novo");
        assert!(!record_envelope(&c, "bob", b"env-1").unwrap(), "segundo: duplicado");
        assert!(record_envelope(&c, "bob", b"env-2").unwrap());
        assert!(record_envelope(&c, "carol", b"env-1").unwrap(), "por contacto");
    }
    {
        // sobrevive a reinicio
        let c = open_database_with_key(&p, &KEY).unwrap();
        let m = load_session_meta(&c, "bob").unwrap().unwrap();
        assert_eq!(m.protocol, "pqxdh");
        assert_eq!((m.last_recv_seq, m.send_seq), (5, 9));
        assert!(!record_envelope(&c, "bob", b"env-1").unwrap(), "replay apos reinicio");
        assert!(load_session_meta(&c, "ninguem").unwrap().is_none());
        delete_session(&c, "bob").unwrap();
        assert!(load_session_meta(&c, "bob").unwrap().is_none());
        assert!(record_envelope(&c, "bob", b"env-1").unwrap());
    }
    std::fs::remove_file(&p).ok();
}

#[test]
fn metadados_invalidos_sao_rejeitados() {
    let p = tmp("metabad");
    let c = open_database_with_key(&p, &KEY).unwrap();
    let m = SessionMeta {
        contact_id: "x".into(),
        peer_identity_public: vec![1u8; 5],
        protocol: "x3dh".into(),
        last_recv_seq: 0,
        send_seq: 0,
    };
    assert!(matches!(save_session_meta(&c, &m), Err(StorageError::CorruptData)));
    assert!(matches!(save_noise_static(&c, &[1u8; 5]), Err(StorageError::CorruptData)));
    std::fs::remove_file(&p).ok();
}

#[test]
fn sessao_ratchet_corrompida_da_erro() {
    let p = tmp("ratbad");
    let c = open_database_with_key(&p, &KEY).unwrap();
    c.execute("INSERT INTO sessions (contact_id, ratchet_state) VALUES ('bob', x'DEADBEEF')", []).unwrap();
    assert!(matches!(load_session(&c, "bob"), Err(StorageError::CorruptRatchetState)));
    assert!(load_session(&c, "inexistente").unwrap().is_none());
    std::fs::remove_file(&p).ok();
}

#[test]
fn ficheiro_de_base_de_dados_corrompido_nao_abre_e_nao_entra_em_panic() {
    let p = tmp("corrupt");
    drop(open_database_with_key(&p, &KEY).unwrap());
    let mut raw = std::fs::read(&p).unwrap();
    for i in 100..200 {
        raw[i] ^= 0xFF;
    }
    std::fs::write(&p, &raw).unwrap();
    assert!(open_database_with_key(&p, &KEY).is_err());
    // ficheiro truncado
    std::fs::write(&p, &raw[..40]).unwrap();
    assert!(open_database_with_key(&p, &KEY).is_err());
    std::fs::remove_file(&p).ok();
}

#[test]
fn wipe_all_apaga_tudo() {
    let p = tmp("wipe");
    let mut c = open_database_with_key(&p, &KEY).unwrap();
    save_noise_static(&c, &[1u8; 32]).unwrap();
    save_one_time_prekey(&c, 1, &DhKeyPair::generate()).unwrap();
    wipe_all(&c).unwrap();
    assert!(load_noise_static(&c).unwrap().is_none());
    assert_eq!(count_one_time_prekeys(&c).unwrap(), 0);
    assert!(take_one_time_prekey(&mut c, 1).is_err());
    std::fs::remove_file(&p).ok();
}

#[cfg(feature = "pq")]
mod pq {
    use super::*;
    use qt_core::pqxdh::PqPreKeyPair;

    #[test]
    fn prekey_pq_sobrevive_a_reinicio_com_a_mesma_identidade() {
        let p = tmp("pq");
        let pub_before;
        {
            let c = open_database_with_key(&p, &KEY).unwrap();
            let kp = PqPreKeyPair::generate();
            pub_before = kp.public_key_bytes();
            save_pq_prekey(&c, 1, &kp).unwrap();
        }
        {
            let c = open_database_with_key(&p, &KEY).unwrap();
            let kp = load_pq_prekey(&c, 1).unwrap().unwrap();
            assert_eq!(kp.public_key_bytes(), pub_before);
            assert!(load_pq_prekey(&c, 2).unwrap().is_none());
            delete_pq_prekey(&c, 1).unwrap();
            assert!(load_pq_prekey(&c, 1).unwrap().is_none());
        }
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn prekey_pq_corrompida_da_erro() {
        let p = tmp("pqbad");
        let c = open_database_with_key(&p, &KEY).unwrap();
        c.execute("INSERT INTO pq_prekeys (id, private) VALUES (1, x'00')", []).unwrap();
        assert!(matches!(load_pq_prekey(&c, 1), Err(StorageError::CorruptPqKey)));
        std::fs::remove_file(&p).ok();
    }
}
