#![no_main]

use betterlite_provider::ollama_chat::ids::RandomIds;
use betterlite_provider::ollama_chat::reply::decode::DecodedReply;
use betterlite_provider::ollama_chat::stream::OllamaChatIterator;
use libfuzzer_sys::fuzz_target;
use std::str;

const NOW: i64 = 1_700_000_000;

fuzz_target!(|bytes: &[u8]| {
    let _reply = DecodedReply::try_from(bytes);
    let mut iterator = OllamaChatIterator::default();
    let _chunk = iterator.chunk_parser(bytes, NOW, &RandomIds);
    if let Ok(line) = str::from_utf8(bytes) {
        let mut stream = OllamaChatIterator::default();
        let _step = stream.next_chunk(line, NOW, &RandomIds);
    }
});
