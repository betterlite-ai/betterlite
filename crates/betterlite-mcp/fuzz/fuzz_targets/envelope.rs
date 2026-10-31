#![no_main]

use betterlite_mcp::envelope::{EnvelopeKeys, open_envelope, open_refresh_envelope};
use chrono::DateTime;
use libfuzzer_sys::fuzz_target;
use secrecy::SecretString;

fuzz_target!(|bytes: &[u8]| {
    let (Ok(candidate), Ok(keys), Some(now)) = (
        std::str::from_utf8(bytes),
        EnvelopeKeys::new(
            SecretString::from("signing-key-that-is-at-least-32-bytes-long"),
            SecretString::from("encryption-key"),
        ),
        DateTime::from_timestamp(1_700_000_000, 0),
    ) else {
        return;
    };
    let _ = open_envelope(candidate, &keys, now);
    let _ = open_refresh_envelope(candidate, &keys, now);
});
