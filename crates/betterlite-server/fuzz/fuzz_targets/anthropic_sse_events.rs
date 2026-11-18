#![no_main]

use betterlite_server::guardrails::anthropic_translation::sse_events::{
    SseChunk, content_block_state, iter_sse_events,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let chunk = SseChunk::Bytes(input.to_vec());
    let _events = iter_sse_events(&chunk);
    let _state = content_block_state(&[chunk]);
});
