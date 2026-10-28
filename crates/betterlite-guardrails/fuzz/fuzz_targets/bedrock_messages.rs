#![no_main]

use betterlite_guardrails::bedrock_message_selection::SelectionContext;
use betterlite_guardrails::bedrock_message_selection::wire::{Message, ModelResponse};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _message = serde_json::from_slice::<Message>(bytes);
    let _response = serde_json::from_slice::<ModelResponse>(bytes);
    let _selection = serde_json::from_slice::<SelectionContext>(bytes);
});
