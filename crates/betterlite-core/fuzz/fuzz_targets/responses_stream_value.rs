#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_stream_primitives::decode::decode_responses_stream_value;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_responses_stream_value(bytes, ApiLimits::default()));
});
