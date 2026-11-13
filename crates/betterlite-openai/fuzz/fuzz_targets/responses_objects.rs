#![no_main]

use betterlite_openai::api::responses_objects::{
    ResponsesApiResponse, decode_responses_objects, wire::WireDecodeLimits,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 0x0004_0000,
        max_items: 10_000,
    };
    let _response = decode_responses_objects::<ResponsesApiResponse>(bytes, limits);
});
