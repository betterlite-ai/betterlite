#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_poll_state::store::decode_responses_poll_state;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_responses_poll_state(bytes, ApiLimits::default()));
});
