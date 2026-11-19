#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_responses_output::decode_messages_responses_out;
use libfuzzer_sys::fuzz_target;

const LIMITS: ApiLimits = ApiLimits {
    max_json_bytes: 4096,
    max_event_bytes: 4096,
    max_items: 100,
};

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_messages_responses_out(bytes, &LIMITS);
});
