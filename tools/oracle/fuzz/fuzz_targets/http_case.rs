#![no_main]

use betterlite_oracle::case::decode_case;
use libfuzzer_sys::fuzz_target;
use std::path::Path;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_case(Path::new("<bytes>"), bytes);
});
