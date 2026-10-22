//! O que o relay recebe de qualquer cliente depois da autenticacao (JSON) e, se for um
//! registo, a verificacao do bundle. Nada disto pode fazer panic no relay.
#![no_main]
use libfuzzer_sys::fuzz_target;
use qt_server::protocol::*;

fuzz_target!(|data: &[u8]| {
    let Ok(m) = deserialize_client_message(data) else { return };
    match m {
        ClientMessage::RegisterPreKeyBundle { user_id, bundle_bytes, .. } => {
            let _ = is_valid_user_id(&user_id);
            let _ = qt_core::identity::verify_published_bundle(&bundle_bytes, &[7u8; 32]);
        }
        ClientMessage::SendMessage { to, .. } => {
            let _ = is_valid_user_id(&to);
        }
        ClientMessage::RegisterPushToken { token, .. } => {
            let _ = is_valid_push_token(&token);
        }
        _ => {}
    }
});
