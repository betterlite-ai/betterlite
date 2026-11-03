#![no_main]

use betterlite_openai::api::responses_shapes::wire::{WireDecodeLimits, decode_responses_shapes};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = WireDecodeLimits {
        max_json_bytes: 1_048_576,
        max_event_bytes: 262_144,
        max_items: 10_000,
    };
    let _decoded = decode_responses_shapes(bytes, limits);
});
