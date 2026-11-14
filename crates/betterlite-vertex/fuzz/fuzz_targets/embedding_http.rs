#![no_main]

use betterlite_tokenizer::count::error::CountError;
use betterlite_vertex::gemini_embedding_call::http::{decode_embedding_http, file_metadata};
use betterlite_vertex::gemini_embedding_records::input::{EmbeddingInput, ResolvedFiles};
use betterlite_vertex::gemini_embedding_records::response::{
    process_batch_reply, process_embed_content_reply,
};
use betterlite_vertex::gemini_embedding_records::usage::TextTokenCounter;
use libfuzzer_sys::fuzz_target;

struct CharacterCount;

impl TextTokenCounter for CharacterCount {
    fn count_text(&self, _model: &str, text: &str) -> Result<u32, CountError> {
        u32::try_from(text.chars().count()).map_err(|_error| CountError::Overflow)
    }
}

fuzz_target!(|bytes: &[u8]| {
    let Ok(reply) = decode_embedding_http(bytes) else {
        return;
    };
    let input = EmbeddingInput::Text(String::from("hello"));
    let _metadata = file_metadata("files/fuzz", &reply);
    let _batch = process_batch_reply(&input, "model", &reply, &CharacterCount);
    let _single = process_embed_content_reply(
        &input,
        "model",
        &reply,
        &ResolvedFiles::new(),
        &CharacterCount,
    );
});
