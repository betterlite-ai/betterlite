#![no_main]

use betterlite_cache::s3_cache::decode_for_fuzzing;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_for_fuzzing(bytes);
});
