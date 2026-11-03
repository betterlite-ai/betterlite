#![no_main]

use std::str;

use betterlite_env::{flag, number};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes) else {
        return;
    };
    let _ = flag("FUZZ", text);
    let _ = number::<i64>("FUZZ", text);
    let _ = number::<u32>("FUZZ", text);
    let _ = number::<f64>("FUZZ", text);
});
