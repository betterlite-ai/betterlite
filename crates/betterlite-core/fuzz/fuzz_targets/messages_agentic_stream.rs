#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_agentic_stream::decode_messages_agentic_stream;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_messages_agentic_stream(bytes, &ApiLimits::default()));
});
