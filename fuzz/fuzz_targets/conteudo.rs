//! Conteudo das mensagens (payload v1: texto, recibos, anexos, convites, grupos,
//! chamadas, fotos, badges...) DEPOIS de decifrado. Um contacto autentico (ou a sua
//! app com um erro) pode mandar qualquer coisa: a Alice cifra os bytes do fuzzer numa
//! sessao real e o Bob recebe-os pelo caminho normal da app.
#![no_main]
use libfuzzer_sys::fuzz_target;
use qt_ffi::{peer_bundle_from_relay_bytes, QtClient};
use std::sync::{Arc, Mutex, OnceLock};

struct Par {
    alice: Arc<QtClient>,
    bob: Arc<QtClient>,
    cid: String,
}

fn cliente(nome: &str, k: u8) -> Arc<QtClient> {
    let p = std::env::temp_dir().join(format!("qt_fuzz_conteudo_{nome}_{}.sqlite", std::process::id()));
    let _ = std::fs::remove_file(&p);
    let c = QtClient::open(p.to_str().unwrap().to_string(), vec![k; 32]).unwrap();
    c.create_identity().unwrap();
    c
}

fn novo_par() -> Par {
    let (alice, bob) = (cliente("alice", 0x11), cliente("bob", 0x22));
    let bundle = peer_bundle_from_relay_bytes(bob.publishable_bundle().unwrap()).unwrap();
    let cid = alice.start_session(bob.export_public_identity().unwrap().public_id, bundle).unwrap();
    Par { alice, bob, cid }
}

fn par() -> &'static Mutex<Par> {
    static P: OnceLock<Mutex<Par>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(novo_par()))
}

fuzz_target!(|data: &[u8]| {
    let mut p = par().lock().unwrap();
    let Ok(sealed) = p.alice.encrypt_sealed(p.cid.clone(), data.to_vec()) else { return };
    // Um conteudo pode mudar o estado do Bob (ex.: "sessao terminada", bloquear): a
    // Alice do teste nao reage como a app, por isso recomeca-se com um par novo.
    if p.bob.receive_sealed(sealed).is_err() {
        *p = novo_par();
    }
});
