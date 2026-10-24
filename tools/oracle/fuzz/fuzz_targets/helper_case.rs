#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let source = String::from_utf8_lossy(bytes);
    let _result = betterlite_oracle::case::parse_helper_case(&source);
});
