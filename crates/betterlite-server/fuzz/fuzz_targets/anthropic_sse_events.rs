#![no_main]

use betterlite_server::guardrails::anthropic_translation::sse_events::{
    SseChunk, content_block_state, iter_sse_events, streaming_has_ended, streaming_string_so_far,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let chunk = SseChunk::Bytes(input.to_vec());
    let _events = iter_sse_events(&chunk);
    let _state = content_block_state(std::slice::from_ref(&chunk));
    let _text = streaming_string_so_far(std::slice::from_ref(&chunk));
    let _ended = streaming_has_ended(&[chunk]);
});
