#![no_main]

use betterlite_logging::s3_transport::error_code::{error_code, is_terminal};
use libfuzzer_sys::fuzz_target;
use std::str::from_utf8;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    let _code = error_code(text);
    let _terminal = is_terminal(400, text);
    let _retried = is_terminal(500, text);
});
