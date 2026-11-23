#![no_main]

use betterlite_server::spend::log_results::query::decode_legacy_spend_query;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let decoded = decode_legacy_spend_query(bytes);
    drop(decoded);
});
