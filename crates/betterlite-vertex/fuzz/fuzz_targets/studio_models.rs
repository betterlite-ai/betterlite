#![no_main]

use betterlite_vertex::studio_identity::decode::decode_studio_models;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _success = decode_studio_models(200, bytes);
    let _failure = decode_studio_models(500, bytes);
});
