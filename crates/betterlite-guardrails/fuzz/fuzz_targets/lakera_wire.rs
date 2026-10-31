#![no_main]

use betterlite_guardrails::lakera_pii::wire::{LakeraResponse, Message};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _messages = serde_json::from_slice::<Vec<Message>>(bytes);
    let _response = serde_json::from_slice::<LakeraResponse>(bytes);
});
