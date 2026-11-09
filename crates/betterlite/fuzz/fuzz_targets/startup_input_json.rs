#![no_main]

use betterlite::startup_input::{decode_headers, decode_worker_config};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8(bytes.to_vec()).unwrap_or_else(|error| {
        error
            .as_bytes()
            .iter()
            .map(|byte| char::from(*byte))
            .collect()
    });
    drop(decode_headers(&text));
    drop(decode_worker_config(&text));
});
