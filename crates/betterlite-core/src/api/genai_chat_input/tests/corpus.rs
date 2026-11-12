use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::prelude::translate_request;
use crate::api::genai_chat_input::input::GenerateContentRequest;
use crate::schema::SchemaLimits;

#[rstest]
fn replays_the_genai_chat_input_corpus(#[files("fuzz/corpus/genai_chat_input/*")] path: PathBuf) {
    let bytes = fs::read(&path).expect("a corpus file");
    if path.file_stem() == Some(OsStr::new("invalid-json")) {
        drop(
            serde_json::from_slice::<GenerateContentRequest>(&bytes)
                .expect_err("a corpus file whose JSON does not parse"),
        );
        return;
    }
    let request = serde_json::from_slice::<GenerateContentRequest>(&bytes)
        .expect("every other corpus file is a GenAI request");
    drop(translate_request(&request, None, SchemaLimits::default()));
}
