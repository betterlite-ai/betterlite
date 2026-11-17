#![no_main]

use betterlite_auth::cli_attribution::claim_map::decode_cli_claim_map;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    drop(decode_cli_claim_map(text));
});
