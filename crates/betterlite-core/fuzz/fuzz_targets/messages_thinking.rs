#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_thinking::response::decode_messages_response;
use betterlite_core::api::messages_thinking::session::decode_messages_thinking;
use libfuzzer_sys::fuzz_target;

const LIMITS: ApiLimits = ApiLimits {
    max_json_bytes: 4096,
    max_event_bytes: 4096,
    max_items: 100,
};

fuzz_target!(|input: &[u8]| {
    let Some((&selector, payload)) = input.split_first() else {
        return;
    };
    if selector.is_multiple_of(2) {
        let _outcome = decode_messages_response(payload, 500, &LIMITS);
        return;
    }
    let _outcome = decode_messages_thinking(payload, &LIMITS);
});
