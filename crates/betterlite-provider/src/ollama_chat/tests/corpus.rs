use std::fs;
use std::str;

use crate::ollama_chat::ids::RandomIds;
use crate::ollama_chat::reply::decode::DecodedReply;
use crate::ollama_chat::stream::OllamaChatIterator;

const NOW: i64 = 1_700_000_000;

#[test]
fn replays_the_ollama_chat_reply_corpus() {
    let seen = fs::read_dir("fuzz/corpus/ollama_chat_reply")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).unwrap();
            let _reply = DecodedReply::try_from(bytes.as_slice());
            let mut iterator = OllamaChatIterator::default();
            let _chunk = iterator.chunk_parser(&bytes, NOW, &RandomIds);
            if let Ok(line) = str::from_utf8(&bytes) {
                let mut stream = OllamaChatIterator::default();
                let _step = stream.next_chunk(line, NOW, &RandomIds);
            }
        })
        .count();
    assert!(seen > 0_usize);
}
