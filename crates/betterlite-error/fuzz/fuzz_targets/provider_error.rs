#![no_main]

use betterlite_error::parse::parse_provider_error;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = parse_provider_error(bytes);
});
