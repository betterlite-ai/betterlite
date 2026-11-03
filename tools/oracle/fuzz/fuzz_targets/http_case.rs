#![no_main]

use betterlite_oracle::case::decode_case;
use libfuzzer_sys::fuzz_target;
use std::path::Path;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_case(Path::new("<bytes>"), bytes);
});
