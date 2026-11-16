#![no_main]

use betterlite_streaming::passthrough_reconstruction::collapse::collapse_pure_text_chunks;
use betterlite_streaming::passthrough_reconstruction::events::{
    RawSseChunk, split_sse_chunk_into_events,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|fragment: String| {
    // LiteLLM's _split_sse_chunk_into_events and _collapse_pure_text_chunks over one relayed fragment.
    let text = RawSseChunk::Text(fragment.clone());
    let _events = split_sse_chunk_into_events(&text);
    let _collapsed = collapse_pure_text_chunks(&[text]);
    let _from_bytes = collapse_pure_text_chunks(&[RawSseChunk::Bytes(fragment.into_bytes())]);
});
