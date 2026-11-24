#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_poll_stream::frame::decode_poll_stream_frame;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_poll_stream_frame(bytes, ApiLimits::default()));
});
