#![no_main]

use betterlite_server::pass_through_logging::stream_fields::{Chunk, extract_message_start_field};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let midpoint = bytes.len().saturating_div(2);
    let (first, second) = bytes.split_at(midpoint);
    let chunks = [Chunk::Bytes(first.to_vec()), Chunk::Bytes(second.to_vec())];
    let _model = extract_message_start_field(&chunks, "model");
    let _id = extract_message_start_field(&chunks, "id");
    if let Ok(text) = str::from_utf8(bytes) {
        let text_chunks = [Chunk::Text(text.to_owned())];
        let _text_model = extract_message_start_field(&text_chunks, "model");
        let _text_id = extract_message_start_field(&text_chunks, "id");
    }
});
