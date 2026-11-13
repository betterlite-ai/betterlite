#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_to_chat_stream::decode_event_line;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_event_line(
        bytes,
        &ApiLimits::default(),
    ));
});
