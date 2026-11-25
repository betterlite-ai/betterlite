#![no_main]

use betterlite_server::shadow_eval_results::query::decode_shadow_query;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_shadow_query(bytes);
});
