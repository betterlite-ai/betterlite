#![no_main]

use betterlite_openai::stream::SseParser;
use betterlite_openai::text_completion_stream::{decode_chat_bytes, decode_chat_events};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _whole = decode_chat_bytes(bytes, "model", true);
    let _unused = decode_chat_bytes(bytes, "", false);
    if let Some((first, rest)) = bytes.split_first() {
        let boundary = usize::from(*first) % (rest.len() + 1);
        let (left, right) = rest.split_at(boundary);
        let mut parser = SseParser::default();
        for piece in [left, right] {
            if let Ok(events) = parser.push(piece) {
                let _ = decode_chat_events(&events, "model", true);
            }
        }
        let _ = decode_chat_events(&parser.finish(), "model", true);
    }
});
