use std::fs;

use crate::ollama_chat::ids::RandomIds;
use crate::ollama_generate::reply::GenerateReply;
use crate::ollama_generate::stream::OllamaGenerateIterator;

const NOW: i64 = 1_700_000_000;

#[test]
fn replays_the_ollama_generate_reply_corpus() {
    let seen = fs::read_dir("fuzz/corpus/ollama_generate_reply")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).unwrap();
            let _reply = GenerateReply::try_from(bytes.as_slice());
            let mut iterator = OllamaGenerateIterator::default();
            let _step = iterator.chunk_parser(&bytes, NOW, &RandomIds);
        })
        .count();
    assert!(seen > 0_usize);
}
