#![no_main]

use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_server::guardrails::anthropic_sse::is_anthropic_sse_stream::is_anthropic_sse_stream;
use betterlite_server::guardrails::anthropic_sse::is_raw_sse_stream::is_raw_sse_stream;
use betterlite_server::guardrails::anthropic_sse::is_sse_error_stream::is_sse_error_stream;
use betterlite_server::guardrails::anthropic_sse::sse_stream::{
    anthropic_message_start, joined_sse_stream, parsed_sse_events,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let chunks = [StreamItem::Bytes(bytes.to_vec())];
    let _raw = is_raw_sse_stream(&chunks);
    let _anthropic = is_anthropic_sse_stream(&chunks);
    let _errors = is_sse_error_stream(&chunks);
    if let Some(stream) = joined_sse_stream(&chunks) {
        let events = parsed_sse_events(&stream);
        let _start = anthropic_message_start(&events);
    }
});
