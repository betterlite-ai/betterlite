#![no_main]

use betterlite_core::api::genai_chat_output::{GenAiStreamState, translate_chunk};
use betterlite_openai::stream::chunk::Chunk;
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 0x0004_0000;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > MAX_INPUT {
        return;
    }
    let Ok(chunk) = serde_json::from_slice::<Chunk>(bytes) else {
        return;
    };
    let mut state = GenAiStreamState::default();
    drop(translate_chunk(&chunk, &mut state));
});
