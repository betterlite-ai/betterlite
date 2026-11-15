#![no_main]

use betterlite_vertex::gemini_response_parts::projection::CallMode;
use betterlite_vertex::gemini_stream::{GeminiStreamDecoder, StreamOptions};
use http::HeaderMap;
use libfuzzer_sys::fuzz_target;
use uuid::Uuid;
fuzz_target!(|bytes: &[u8]| {
    let options = StreamOptions {
        now: 1_700_000_000,
        call_mode: CallMode::Tools,
    };
    let ids = Box::new(|| Uuid::from_u128(0));
    let mut decoder = GeminiStreamDecoder::new(options, &HeaderMap::new(), ids);
    // a growing step puts fragment boundaries where the network could have split the bytes
    let mut rest = bytes;
    for step in 1..bytes.len().max(1) {
        let Some((fragment, tail)) = rest.split_at_checked(step.min(rest.len())) else {
            break;
        };
        drop(decoder.push(fragment));
        rest = tail;
    }
    drop(decoder.push(rest));
    while let Ok(Some(_chunk)) = decoder.finish() {}
});
