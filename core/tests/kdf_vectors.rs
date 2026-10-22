//! Regressao (22/10/2026): zerar os valores intermedios das KDF e do DH ratchet nao
//! pode mudar nenhum resultado (as sessoes guardadas e as apps antigas dependem dele).
//! Todos os valores abaixo foram produzidos pelo codigo ANTERIOR a essa mudanca.

use qt_core::primitives::{kdf_chain_key, kdf_root_key, DhKeyPair};
use qt_core::ratchet::{EncryptedMessage, RatchetState};
use x25519_dalek::{PublicKey, StaticSecret};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

fn msg(dh: &str, pn: u32, n: u32, ct: &str) -> EncryptedMessage {
    let dh: [u8; 32] = unhex(dh).try_into().unwrap();
    EncryptedMessage { dh_public: PublicKey::from(dh), pn, n, ciphertext: unhex(ct) }
}

const AD: &[u8] = b"AD-teste";
const SHARED: [u8; 32] = [0x55; 32];
const SPK: [u8; 32] = [0x44; 32];

#[test]
fn vetores_fixos_das_kdf_do_ratchet() {
    let (root, chain) = kdf_root_key(&[0x11; 32], &[0x22; 32]);
    let (mk, next) = kdf_chain_key(&[0x33; 32]);
    assert_eq!(hex(&root), "06128f848cf9ee5c1e59be10be0ba0ebbe5e973da81c06cce8f759636b97da9f");
    assert_eq!(hex(&chain), "fcbd68010bb5aa82ca7e43fe6a71cd60f4083439388100930dd4e1c1e29327fe");
    assert_eq!(hex(&mk), "17e95e199a5aa841caa71f66270766f554102a5a1b87ce2b9f7da18054726ec7");
    assert_eq!(hex(&next), "cf8312e657d316718855ef4cf4099694ea347cca2bf8c88886d7f924a02b277f");
}

/// Mensagens cifradas pelo codigo antigo (iniciador) decifram-se no novo (respondedor),
/// fora de ordem (passa pelas chaves saltadas).
#[test]
fn mensagens_do_codigo_antigo_decifram_no_novo() {
    let spk_priv = StaticSecret::from(SPK);
    let spk = DhKeyPair { public: PublicKey::from(&spk_priv), private: spk_priv };
    let mut b = RatchetState::init_as_responder(SHARED, spk).with_associated_data(AD.to_vec());
    let dh = "6b6d75897dcd454408d85e4d550342640c755399a35f49b172f6515ef114f302";
    let m1 = msg(dh, 0, 1, "b970d6fdb649eb08983b75d7ce69c30a2edfe7adccd91bc903e9b44d3cea6dfeed68a1");
    let m0 = msg(dh, 0, 0, "4403fa4205a4436e2a654205634f752305e18ee89b232d154eb69016314dafcdc4d29056");
    assert_eq!(b.decrypt(&m1).unwrap(), b"segunda");
    assert_eq!(b.decrypt(&m0).unwrap(), b"primeira");
}

/// Estado guardado pelo codigo antigo (iniciador) decifra a resposta do codigo antigo
/// (passa pelo passo de DH ratchet do iniciador).
#[test]
fn estado_guardado_antigo_decifra_a_resposta() {
    let mut a = RatchetState::from_bytes(&unhex(concat!(
        "26ead788b9a5f37682f888c3e366896fb33d7ffd058e330b532fc82e97107b9f6a1b7862cb4d0725aea0c557001d91f1",
        "f6e71cab8db213133dea39870b70debeb3aafd4aba0704376b5731e26f3762eae9d7983050e2d4c3c4a3ee4c76b34927",
        "01ff2ee45601ec1b67310c7790404585ae697331eee1c1f8cf2419731c1fff3e6b018a7ad3f62a019cdcaf1d84cc06ff",
        "cf976837d2da3b5b47b7dacb03b5b53263990000000001000000000000000000000000000841442d7465737465"
    )))
    .unwrap();
    let r = msg(
        "1b64c14258820df9152f69dd9e190d7fb1487154342214223338832087ebf719",
        0,
        0,
        "8ed4b2ffd9d736b594d6b0252b394d0b70b34576cf6d7d6e46bb0f557f6924184906c48e",
    );
    assert_eq!(a.decrypt(&r).unwrap(), b"resposta");
    // E a conversa continua nos dois sentidos com o codigo novo.
    let m = a.encrypt(b"depois").unwrap();
    assert_eq!(m.n, 0);
}
