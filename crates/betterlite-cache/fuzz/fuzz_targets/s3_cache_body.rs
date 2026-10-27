#![no_main]

use betterlite_cache::s3_cache::body::decode_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_body(bytes);
});
