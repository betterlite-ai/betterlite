#![no_main]

use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::stream_chunk_events::anthropic_stream_chunk_events;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _events = anthropic_stream_chunk_events(&StreamItem::Bytes(bytes.to_vec()));
});
