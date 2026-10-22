//! Estado do ratchet lido do disco (pode estar corrompido): ou e recusado, ou volta a
//! dar exatamente os mesmos bytes.
#![no_main]
use libfuzzer_sys::fuzz_target;
use qt_core::ratchet::RatchetState;

fuzz_target!(|data: &[u8]| {
    if let Some(s) = RatchetState::from_bytes(data) {
        let again = s.to_bytes();
        let s2 = RatchetState::from_bytes(&again).expect("o que se escreve tem de se ler");
        assert_eq!(s2.to_bytes(), again);
    }
    let _ = qt_core::pqxdh::PqPreKeyPair::from_bytes(data);
});
