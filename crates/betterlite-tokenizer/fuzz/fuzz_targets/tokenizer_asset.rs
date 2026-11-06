#![no_main]
use betterlite_tokenizer::engine::huggingface::HuggingFaceEngine;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _vocabulary = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, bytes);
    let _tokenizer = HuggingFaceEngine::from_bytes(bytes);
});
