#![no_main]

use std::str;

use betterlite_config::sections::budget_reset::clock::decode_reset_clock;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = str::from_utf8(bytes) {
        drop(decode_reset_clock(text));
    }
});
