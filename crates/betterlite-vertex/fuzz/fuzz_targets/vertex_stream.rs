#![no_main]

use betterlite_vertex::gemini_response_parts::projection::CallMode;
use betterlite_vertex::gemini_stream::{GeminiStreamDecoder, StreamOptions, UuidV4};
use http::HeaderMap;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let mut decoder = GeminiStreamDecoder::new(
        StreamOptions {
            now: 1_700_000_000,
            call_mode: CallMode::Tools,
        },
        &HeaderMap::new(),
        Box::new(UuidV4),
    );
    // a growing step puts fragment boundaries where the network could have split the bytes
    let mut rest = bytes;
    let mut step = 1_usize;
    while let Some((fragment, tail)) = rest.split_at_checked(step.min(rest.len())) {
        drop(decoder.push(fragment));
        rest = tail;
        step = step.saturating_add(1);
        if rest.is_empty() {
            break;
        }
    }
    while let Ok(Some(_chunk)) = decoder.finish() {}
});
