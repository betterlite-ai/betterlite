#![no_main]

use betterlite_auth::jwt_claims::TypedValue;
use betterlite_mcp::sso_assertions::assertion::{assertion_from_sso_login, decode_payload};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let text = lossy_text(bytes);
    let Ok(quoted) = serde_json::to_string(&text) else {
        return;
    };
    let Ok(id_token) = serde_json::from_str::<TypedValue>(&quoted) else {
        return;
    };
    let _assertion = assertion_from_sso_login(Some(&id_token), None);
    let _payload = decode_payload(&text);
});

// the provider's bytes and a database row's bytes both reach the decoders as text, so invalid UTF-8 is replaced rather than skipped
fn lossy_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}
