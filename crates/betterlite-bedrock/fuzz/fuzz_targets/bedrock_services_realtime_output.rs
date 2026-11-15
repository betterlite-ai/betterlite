#![no_main]

use std::str;

use betterlite_bedrock::realtime_output::metadata::decode_generation_metadata;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    if let Ok(text) = str::from_utf8(input) {
        drop(decode_generation_metadata(text));
    }
});
