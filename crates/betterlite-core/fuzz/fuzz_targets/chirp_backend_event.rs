#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::vertex_chirp::decode::decode_chirp_backend_event;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_chirp_backend_event(data, ApiLimits::default()));
});
