#![no_main]

use betterlite_guardrails::lasso_masking::wire::{Message, ModelResponseShape};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _messages = serde_json::from_slice::<Vec<Message>>(bytes);
    let _response = serde_json::from_slice::<ModelResponseShape>(bytes);
});
