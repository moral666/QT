//! Bundle vindo do relay (a app confia so na assinatura): formato v3/v4 e a resposta
//! `PreKeyBundle` com a one-time prekey.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = qt_ffi::peer_bundle_from_relay_bytes(data.to_vec());
    if let Ok(b) = qt_core::identity::parse_bundle_v4(data) {
        let _ = qt_core::identity::verify_bundle_v4(&b);
    }
    let signing: [u8; 32] = data.get(..32).and_then(|s| s.try_into().ok()).unwrap_or([0; 32]);
    let _ = qt_core::identity::verify_published_bundle(data, &signing);
    let _ = qt_core::pqxdh::pq_public_key_from_bytes(data);
    let _ = qt_core::pqxdh::pq_ciphertext_from_bytes(data);
});
