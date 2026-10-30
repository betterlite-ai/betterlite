#![no_main]

use betterlite_anthropic::api::messages_errors::{
    WireDecodeLimits, decode_messages_errors, transform_messages_error,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let Some((&selector, payload)) = input.split_first() else {
        return;
    };
    match selector % 2 {
        0 => {
            let _ = decode_messages_errors(
                payload,
                WireDecodeLimits {
                    max_json_bytes: 64 * 1024,
                    max_event_bytes: 64 * 1024,
                    max_items: 256,
                },
            );
        }
        _ => {
            if let Ok(text) = std::str::from_utf8(payload) {
                let _ = transform_messages_error(500, text, Some("fuzz"));
            }
        }
    }
});
