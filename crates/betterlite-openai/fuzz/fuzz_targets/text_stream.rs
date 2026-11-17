#![no_main]

use betterlite_openai::stream::SseParser;
use betterlite_openai::text_completion_stream::decode_chat_events;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    // the whole buffer read at once, the way the corpus replay decodes it
    let mut whole = SseParser::default();
    if let Ok(events) = whole.push(bytes) {
        let _model_chunks = decode_chat_events(&events, "model", true);
        let _unasked_chunks = decode_chat_events(&events, "", false);
    }
    let _tail_chunks = decode_chat_events(&whole.finish(), "model", true);
    if let Some((first, rest)) = bytes.split_first() {
        let units = rest.len().saturating_add(1);
        let Some(boundary) = usize::from(*first).checked_rem(units) else {
            return;
        };
        let mut parser = SseParser::default();
        for piece in <[&[u8]; 2]>::from(rest.split_at(boundary)) {
            if let Ok(events) = parser.push(piece) {
                let _piece_chunks = decode_chat_events(&events, "model", true);
            }
        }
        let _split_tail_chunks = decode_chat_events(&parser.finish(), "model", true);
    }
});
