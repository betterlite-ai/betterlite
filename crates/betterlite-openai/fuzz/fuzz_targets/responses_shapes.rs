#![no_main]

use betterlite_openai::api::responses_shapes::wire::{WireDecodeLimits, decode_responses_shapes};
use betterlite_openai::api::responses_shapes::{DeleteResponseResult, OutputText};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 0x0004_0000,
        max_items: 10_000,
    };
    let _deletion = decode_responses_shapes::<DeleteResponseResult>(bytes, limits);
    let _decoded = decode_responses_shapes::<OutputText>(bytes, limits);
});
