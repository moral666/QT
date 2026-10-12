//! Forward secrecy, post-compromise security e robustez do Double Ratchet.
//!
//! Modelo de atacante: captura TODO o trafego de rede (todos os ciphertexts)
//! e, num dado instante, obtem uma copia do ESTADO do ratchet de um dos
//! lados (`to_bytes`). Pergunta: o que consegue decifrar?

use qt_core::primitives::DhKeyPair;
use qt_core::ratchet::{EncryptedMessage, RatchetState};

fn par() -> (RatchetState, RatchetState) {
    let shared = [7u8; 32];
    let bob_spk = DhKeyPair::generate();
    let alice = RatchetState::init_as_initiator(shared, bob_spk.public);
    let bob = RatchetState::init_as_responder(shared, bob_spk);
    (alice, bob)
}
fn roubar(s: &RatchetState) -> RatchetState {
    RatchetState::from_bytes(&s.to_bytes()).expect("estado roubado")
}

#[test]
fn estado_comprometido_nao_decifra_mensagens_anteriores() {
    let (mut alice, mut bob) = par();
    let c: Vec<EncryptedMessage> = (0..3u8).map(|i| alice.encrypt(&[i; 16]).unwrap()).collect();
    for (i, m) in c.iter().enumerate() {
        assert_eq!(bob.decrypt(m).unwrap(), vec![i as u8; 16]);
    }
    // O atacante tem todos os ciphertexts e AGORA rouba o estado de Bob...
    let mut roubado_bob = roubar(&bob);
    for m in &c {
        assert!(roubado_bob.decrypt(m).is_err(), "FS violada: estado atual decifrou mensagem antiga");
    }
    // ...e o de Alice (que enviou as mensagens): tambem nao as consegue reler.
    let mut roubado_alice = roubar(&alice);
    for m in &c {
        assert!(roubado_alice.decrypt(m).is_err());
    }
}

#[test]
fn forward_secrecy_atravessa_viragens_do_dh_ratchet() {
    let (mut alice, mut bob) = par();
    let mut capturadas = Vec::new();
    for ronda in 0..4u8 {
        let m = alice.encrypt(&[ronda; 8]).unwrap();
        capturadas.push(m.clone());
        assert_eq!(bob.decrypt(&m).unwrap(), vec![ronda; 8]);
        let r = bob.encrypt(&[100 + ronda; 8]).unwrap();
        capturadas.push(r.clone());
        assert_eq!(alice.decrypt(&r).unwrap(), vec![100 + ronda; 8]);
    }
    for (nome, estado) in [("alice", roubar(&alice)), ("bob", roubar(&bob))] {
        let mut e = estado;
        for m in &capturadas {
            assert!(e.decrypt(m).is_err(), "FS violada com estado de {nome}");
        }
    }
}

#[test]
fn avanco_do_ratchet_gera_novas_chaves() {
    let (mut alice, mut bob) = par();
    let pt = b"mesmo plaintext";
    let m0 = alice.encrypt(pt).unwrap();
    let m1 = alice.encrypt(pt).unwrap();
    // Mesma cadeia: n avanca, ciphertexts distintos (chave de mensagem nova + nonce novo).
    assert_eq!((m0.n, m1.n), (0, 1));
    assert_ne!(m0.ciphertext, m1.ciphertext);
    assert_eq!(m0.dh_public.as_bytes(), m1.dh_public.as_bytes());
    bob.decrypt(&m0).unwrap();
    bob.decrypt(&m1).unwrap();
    // Viragem: Bob responde -> Alice processa -> a proxima mensagem de Alice usa NOVA chave DH.
    let r = bob.encrypt(b"resposta").unwrap();
    alice.decrypt(&r).unwrap();
    let m2 = alice.encrypt(pt).unwrap();
    assert_ne!(m2.dh_public.as_bytes(), m0.dh_public.as_bytes(), "o DH ratchet devia ter avancado");
    assert_eq!(m2.n, 0, "contador reinicia na nova cadeia");
    assert_ne!(m2.ciphertext, m0.ciphertext);
    // E o estado persistido mudou a cada passo.
    let s1 = alice.to_bytes();
    let _ = alice.encrypt(pt).unwrap();
    assert_ne!(s1, alice.to_bytes());
}

#[test]
fn message_key_usada_nao_pode_ser_reutilizada() {
    let (mut alice, mut bob) = par();
    let c1 = alice.encrypt(b"um").unwrap();
    let c2 = alice.encrypt(b"dois").unwrap();
    let c3 = alice.encrypt(b"tres").unwrap();
    assert_eq!(bob.decrypt(&c1).unwrap(), b"um");
    assert!(bob.decrypt(&c1).is_err(), "replay da mesma mensagem");
    // fora de ordem: c3 primeiro, depois c2 (usa a chave guardada, uma unica vez)
    assert_eq!(bob.decrypt(&c3).unwrap(), b"tres");
    assert_eq!(bob.decrypt(&c2).unwrap(), b"dois");
    assert!(bob.decrypt(&c2).is_err(), "chave 'skipped' so pode ser usada uma vez");
    assert!(bob.decrypt(&c3).is_err());
    // o estado persistido nao guarda chaves ja consumidas: um clone nao as reutiliza
    let mut clone = roubar(&bob);
    assert!(clone.decrypt(&c1).is_err() && clone.decrypt(&c2).is_err() && clone.decrypt(&c3).is_err());
}

#[test]
fn limitacao_chaves_puladas_ficam_no_estado_ate_serem_usadas() {
    // LIMITACAO DOCUMENTADA: se c1 e c2 ainda nao chegaram, as suas chaves
    // ficam guardadas no estado (necessario para as aceitar mais tarde).
    // Quem roubar o estado NESSE momento pode decifrar essas mensagens
    // (nao as ja consumidas). E inerente ao Double Ratchet com mensagens
    // fora de ordem (limitado por MAX_SKIP).
    let (mut alice, mut bob) = par();
    let c1 = alice.encrypt(b"pendente-1").unwrap();
    let c2 = alice.encrypt(b"pendente-2").unwrap();
    let c3 = alice.encrypt(b"entregue").unwrap();
    bob.decrypt(&c3).unwrap();
    let mut roubado = roubar(&bob);
    assert_eq!(roubado.decrypt(&c1).unwrap(), b"pendente-1");
    assert_eq!(roubado.decrypt(&c2).unwrap(), b"pendente-2");
    assert!(roubar(&bob).decrypt(&c3).is_err(), "a ja entregue continua protegida");
}

#[test]
fn post_compromise_security_cura_apos_novo_passo_dh() {
    let (mut alice, mut bob) = par();
    let roubado_antes = bob.to_bytes(); // comprometimento ANTES da conversa
    let m1 = alice.encrypt(b"m1").unwrap();
    bob.decrypt(&m1).unwrap();
    let r1 = bob.encrypt(b"r1").unwrap(); // Bob gera DH nova (que o atacante nao conhece)
    alice.decrypt(&r1).unwrap();
    let m2 = alice.encrypt(b"m2 - depois da cura").unwrap();

    let mut atacante = RatchetState::from_bytes(&roubado_antes).unwrap();
    assert_eq!(atacante.decrypt(&m1).unwrap(), b"m1", "(o comprometimento inicial expoe a 1.a mensagem)");
    assert!(atacante.decrypt(&m2).is_err(), "PCS: mensagens apos o novo passo DH ja nao sao decifraveis");
    // O destinatario legitimo continua a decifrar.
    assert_eq!(bob.decrypt(&m2).unwrap(), b"m2 - depois da cura");
}

#[test]
fn decrypt_falhado_nao_altera_o_estado() {
    let (mut alice, mut bob) = par();
    let c1 = alice.encrypt(b"legitima").unwrap();
    let antes = bob.to_bytes();

    // Atacante sem chaves injeta mensagens forjadas (chave DH nova, n alto, lixo).
    for n in [0u32, 1, 5, 999, 1_000_000] {
        let forjada = EncryptedMessage {
            dh_public: DhKeyPair::generate().public,
            n,
            ciphertext: vec![0xAB; 48],
        };
        assert!(bob.decrypt(&forjada).is_err());
        assert_eq!(bob.to_bytes(), antes, "estado alterado por mensagem forjada (n={n})");
    }
    // Ciphertext adulterado da cadeia atual
    let mut adulterada = c1.clone();
    adulterada.ciphertext[20] ^= 1;
    assert!(bob.decrypt(&adulterada).is_err());
    assert_eq!(bob.to_bytes(), antes);
    // n adulterado (AAD)
    let mut n_errado = c1.clone();
    n_errado.n += 1;
    assert!(bob.decrypt(&n_errado).is_err());
    assert_eq!(bob.to_bytes(), antes);
    // A sessao legitima continua intacta.
    assert_eq!(bob.decrypt(&c1).unwrap(), b"legitima");
}

#[test]
fn salto_excessivo_e_rejeitado_sem_estragar_a_sessao() {
    let (mut alice, mut bob) = par();
    let mut ultima = alice.encrypt(b"x").unwrap();
    for _ in 0..1002 {
        ultima = alice.encrypt(b"x").unwrap();
    }
    let antes = bob.to_bytes();
    assert!(bob.decrypt(&ultima).is_err(), "n > MAX_SKIP deve ser rejeitado");
    assert_eq!(bob.to_bytes(), antes);
}

#[test]
fn limitacao_fora_de_ordem_entre_cadeias_dh_perde_a_mensagem() {
    // LIMITACAO CONHECIDA (formato de fio atual): o cabecalho nao leva `pn`
    // (tamanho da cadeia anterior, como na especificacao do Signal), por isso,
    // quando chega uma mensagem de uma cadeia DH NOVA antes de mensagens
    // atrasadas da cadeia ANTERIOR, estas ultimas nao se conseguem decifrar.
    // O estado nao e corrompido e a sessao continua. Corrigir exige mudar o formato.
    let (mut alice, mut bob) = par();
    let a1 = alice.encrypt(b"a1").unwrap();
    let a2 = alice.encrypt(b"a2").unwrap(); // vai chegar atrasada
    bob.decrypt(&a1).unwrap();
    let b1 = bob.encrypt(b"b1").unwrap();
    alice.decrypt(&b1).unwrap();
    let a3 = alice.encrypt(b"a3").unwrap(); // cadeia DH nova
    assert_eq!(bob.decrypt(&a3).unwrap(), b"a3");
    assert!(bob.decrypt(&a2).is_err(), "a2 (cadeia antiga, atrasada) perde-se: limitacao conhecida");
    // a sessao nao ficou danificada:
    let a4 = alice.encrypt(b"a4").unwrap();
    assert_eq!(bob.decrypt(&a4).unwrap(), b"a4");
}

#[test]
fn fora_de_ordem_dentro_da_mesma_cadeia_funciona() {
    let (mut alice, mut bob) = par();
    let ms: Vec<_> = (0..10u8).map(|i| alice.encrypt(&[i]).unwrap()).collect();
    for i in [9usize, 0, 5, 3, 1, 2, 8, 4, 7, 6] {
        assert_eq!(bob.decrypt(&ms[i]).unwrap(), vec![i as u8]);
    }
    for m in &ms {
        assert!(bob.decrypt(m).is_err());
    }
}

#[test]
fn estado_serializado_corrompido_nunca_entra_em_panic() {
    let (mut alice, mut bob) = par();
    let m = alice.encrypt(b"x").unwrap();
    bob.decrypt(&m).unwrap();
    let r = bob.encrypt(b"y").unwrap();
    alice.decrypt(&r).unwrap();
    let bytes = alice.to_bytes();
    // restaura e continua
    let mut restaurado = RatchetState::from_bytes(&bytes).unwrap();
    let m2 = restaurado.encrypt(b"depois do restore").unwrap();
    assert_eq!(bob.decrypt(&m2).unwrap(), b"depois do restore");
    // truncagens e bit-flips: ou None ou um estado qualquer, nunca panic
    for len in 0..bytes.len() {
        let _ = RatchetState::from_bytes(&bytes[..len]);
    }
    for i in 0..bytes.len() {
        let mut b = bytes.clone();
        b[i] ^= 0xFF;
        let _ = RatchetState::from_bytes(&b);
    }
    // contagem de chaves puladas gigante (tentativa de alocacao enorme)
    let mut b = bytes.clone();
    let off = b.len() - 4;
    b[off..].copy_from_slice(&u32::MAX.to_be_bytes());
    let _ = RatchetState::from_bytes(&b);
}
