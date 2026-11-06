#![no_main]
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let Some(data) = assets::bundled(Encoding::P50kBase) else {
        return;
    };
    let Ok(engine) = TiktokenEngine::from_bpe_file(Encoding::P50kBase, &data) else {
        return;
    };
    let ids: Vec<u32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| u32::from_le_bytes(*chunk))
        .collect();
    let _decoded = engine.decode(&ids);
});
