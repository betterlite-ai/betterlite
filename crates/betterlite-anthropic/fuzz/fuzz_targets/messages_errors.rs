#![no_main]

use betterlite_anthropic::api::messages_errors::AnthropicExceptionMapping;
use betterlite_anthropic::api::messages_errors::decode::{
    WireDecodeLimits, decode_messages_errors,
};
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|input: &[u8]| {
    let Some((&selector, payload)) = input.split_first() else {
        return;
    };
    let limits = WireDecodeLimits {
        max_json_bytes: 4096,
        max_event_bytes: 4096,
        max_items: 100,
    };
    if selector.is_multiple_of(2) {
        let _outcome = decode_messages_errors(payload, limits);
        return;
    }
    if let Ok(text) = str::from_utf8(payload) {
        let _outcome = AnthropicExceptionMapping::transform_to_anthropic_error(
            500,
            text,
            Some("fuzz"),
            limits,
        );
    }
});
