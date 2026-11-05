#![no_main]

use std::str;

use betterlite_env::{flag, number};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes) else {
        return;
    };
    let _flag = flag("FUZZ", text);
    let _signed = number::<i64>("FUZZ", text);
    let _unsigned = number::<u32>("FUZZ", text);
    let _float = number::<f64>("FUZZ", text);
});
