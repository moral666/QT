//! Envelope selado e respostas do relay, como a app os recebe (qualquer relay ou
//! atacante na rede pode mandar bytes arbitrarios). So pode dar erro, nunca panic.
#![no_main]
use libfuzzer_sys::fuzz_target;
use qt_ffi::QtClient;
use std::sync::{Arc, OnceLock};

fn bob() -> &'static Arc<QtClient> {
    static B: OnceLock<Arc<QtClient>> = OnceLock::new();
    B.get_or_init(|| {
        let p = std::env::temp_dir().join(format!("qt_fuzz_envelope_{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let c = QtClient::open(p.to_str().unwrap().to_string(), vec![0x22; 32]).unwrap();
        c.create_identity().unwrap();
        c
    })
}

fuzz_target!(|data: &[u8]| {
    let b = bob();
    let _ = b.receive_sealed(data.to_vec());
    let _ = b.decrypt_sealed(data.to_vec());
    let _ = qt_ffi::relay_parse_response(data.to_vec());
});
