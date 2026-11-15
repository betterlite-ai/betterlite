use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::gemini_embedding_call::http::{decode_embedding_http, file_metadata};
use crate::gemini_embedding_records::input::{EmbeddingInput, ResolvedFiles};
use crate::gemini_embedding_records::response::{process_batch_reply, process_embed_content_reply};

use super::support::CharacterCount;

#[rstest]
fn replays_the_embedding_http_corpus(#[files("fuzz/corpus/embedding_http/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(reply) = decode_embedding_http(&bytes) else {
        return;
    };
    let input = EmbeddingInput::Text(String::from("hello"));
    drop(file_metadata("files/fuzz", &reply));
    drop(process_batch_reply(
        &input,
        "model",
        &reply,
        &CharacterCount,
    ));
    drop(process_embed_content_reply(
        &input,
        "model",
        &reply,
        &ResolvedFiles::new(),
        &CharacterCount,
    ));
}
