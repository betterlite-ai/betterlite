#![no_main]

use betterlite_provider::ollama_chat::ids::RandomIds;
use betterlite_provider::ollama_generate::reply::GenerateReply;
use betterlite_provider::ollama_generate::stream::OllamaGenerateIterator;
use libfuzzer_sys::fuzz_target;

const NOW: i64 = 1_700_000_000;

fuzz_target!(|bytes: &[u8]| {
    let _reply = GenerateReply::try_from(bytes);
    let mut iterator = OllamaGenerateIterator::default();
    let _step = iterator.chunk_parser(bytes, NOW, &RandomIds);
});
