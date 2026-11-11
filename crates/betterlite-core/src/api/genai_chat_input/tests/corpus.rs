use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::api::genai_chat_input::{GenerateContentRequest, translate_generate_content};
use crate::schema::SchemaLimits;

#[rstest]
fn replays_the_genai_chat_input_corpus(#[files("fuzz/corpus/genai_chat_input/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("a corpus file");
    if let Ok(request) = serde_json::from_slice::<GenerateContentRequest>(&bytes) {
        drop(translate_generate_content(
            &request,
            None,
            SchemaLimits::default(),
        ));
    }
}
