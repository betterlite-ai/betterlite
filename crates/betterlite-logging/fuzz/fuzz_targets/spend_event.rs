#![no_main]

use betterlite_logging::spend_event::decode::decode_spend_event;
use betterlite_logging::spend_event::restore::callback_args;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(event) = decode_spend_event(bytes) {
        if let Ok(encoded) = serde_json::to_vec(&event) {
            drop(decode_spend_event(&encoded));
        }
        drop(callback_args(event));
    }
});
