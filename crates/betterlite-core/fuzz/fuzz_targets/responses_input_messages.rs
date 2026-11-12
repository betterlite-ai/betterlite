#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_input_messages::thinking::decode_thinking_blocks;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_thinking_blocks(data, ApiLimits::default()));
    let bounded = ApiLimits {
        max_json_bytes: 32,
        ..ApiLimits::default()
    };
    drop(decode_thinking_blocks(data, bounded));
});
