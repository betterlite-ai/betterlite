#![no_main]

use betterlite_server::shadow_eval_lifecycle::body::decode_shadow_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_shadow_body(bytes);
});
