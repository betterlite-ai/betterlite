#![no_main]

use betterlite_provider::databricks_auth::oauth::parse_oauth_token_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Some((&status, body)) = bytes.split_first() else {
        return;
    };
    let Ok(body) = std::str::from_utf8(body) else {
        return;
    };
    let _token = parse_oauth_token_reply(u16::from(status), body);
});
