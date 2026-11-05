#![no_main]

use std::str::from_utf8;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_secret_managers::oidc::token_ttl::oidc_token_cache_ttl;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(token) = from_utf8(bytes) {
        let _ttl = oidc_token_cache_ttl(token, 3600, 1_700_000_000);
    }
    let wrapped = format!("e30.{}.c2ln", URL_SAFE_NO_PAD.encode(bytes));
    let _ttl = oidc_token_cache_ttl(&wrapped, 3600, 1_700_000_000);
});
