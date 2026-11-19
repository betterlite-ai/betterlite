#![no_main]

use betterlite_provider::legacy_dispatch::custom::decode_custom_output;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_custom_output(bytes);
});
