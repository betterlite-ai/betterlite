#![no_main]

use std::str;
use std::time::Duration;

use betterlite_core::api::request_form::scalars::scalar_float;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes) else {
        return;
    };
    let Some(seconds) = scalar_float(text) else {
        return;
    };
    let _converted = Duration::try_from_secs_f64(seconds);
});
