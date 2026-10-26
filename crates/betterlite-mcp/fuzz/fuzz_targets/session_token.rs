#![no_main]

use betterlite_mcp::session_tokens::open_session_token;
use betterlite_mcp::types::{
    AsymmetricSessionKeys, SessionKeys, SessionSigningKeys, SessionTokenKind,
};
use chrono::DateTime;
use libfuzzer_sys::fuzz_target;
use secrecy::SecretString;
fuzz_target!(|bytes: &[u8]| {
    let private = SecretString::from(include_str!("../../src/fixtures/session-private.pem"));
    let (Ok(candidate), Ok(symmetric), Ok(asymmetric), Some(now)) = (
        std::str::from_utf8(bytes),
        SessionKeys::new(SecretString::from("0123456789abcdef0123456789abcdef")),
        AsymmetricSessionKeys::new(&private, "current".to_owned(), Vec::new()),
        DateTime::from_timestamp(1_767_225_600, 0),
    ) else {
        return;
    };
    let keys = [
        SessionSigningKeys::Symmetric(symmetric),
        SessionSigningKeys::Asymmetric(asymmetric),
    ];
    for signing in &keys {
        let _ = open_session_token(candidate, SessionTokenKind::Session, signing, now);
        let _ = open_session_token(candidate, SessionTokenKind::Refresh, signing, now);
    }
});
