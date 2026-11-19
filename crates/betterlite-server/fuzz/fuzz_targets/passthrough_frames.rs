#![no_main]

use std::str::from_utf8;

use betterlite_server::passthrough::websocket::{json_object_frame, rewrite_live_setup_model};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(json_object_frame(bytes));
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    drop(rewrite_live_setup_model(
        text,
        Some(&|model: &str| format!("projects/p/locations/l/models/{model}")),
    ));
});
