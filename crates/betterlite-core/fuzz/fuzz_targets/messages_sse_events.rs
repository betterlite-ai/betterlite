#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_sse_events::decode_messages_sse_chunk;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_messages_sse_chunk(bytes, &ApiLimits::default()));
});
