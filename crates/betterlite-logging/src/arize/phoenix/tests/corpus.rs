#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::arize::phoenix::parsing::parse_prompt_data;
use crate::arize::phoenix::records::PromptVersionEnvelope;

#[rstest]
fn replays_the_phoenix_response_corpus(#[files("fuzz/corpus/phoenix_response/*")] path: PathBuf) {
    let body = fs::read(&path).unwrap();
    let Ok(envelope) = serde_json::from_slice::<PromptVersionEnvelope>(&body) else {
        return;
    };
    let Some(data) = envelope.data else {
        return;
    };
    let _template = parse_prompt_data(data, "fuzz");
}
