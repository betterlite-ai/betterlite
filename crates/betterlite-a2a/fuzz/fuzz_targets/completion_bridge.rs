#![no_main]

use betterlite_a2a::completion_bridge::wire::{
    A2AMessage, CompletionParams, MessageSendParams, ModelResponse,
};
use betterlite_a2a::completion_bridge::{apply_forward_metadata, to_openai_messages};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(message) = serde_json::from_slice::<A2AMessage>(bytes) {
        let _messages = to_openai_messages(&message);
        let mut completion = CompletionParams::default();
        apply_forward_metadata(&mut completion, &message, None);
    }
    let _completion = serde_json::from_slice::<CompletionParams>(bytes);
    let _send = serde_json::from_slice::<MessageSendParams>(bytes);
    let _response = serde_json::from_slice::<ModelResponse>(bytes);
});
