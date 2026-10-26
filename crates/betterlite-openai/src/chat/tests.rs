#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::ChatRequest;

#[rstest]
#[case::basic(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#)]
#[case::tools(r#"{"model":"gpt-4o","messages":[],"tools":[]}"#)]
fn parses_chat_requests(#[case] body: &str) {
    let request: ChatRequest = serde_json::from_str(body).expect("valid request");
    assert!(!request.model.is_empty());
}

#[test]
fn replays_the_chat_request_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/chat_request");
    let entries = fs::read_dir(corpus).expect("corpus exists");
    for entry in entries {
        let path = entry.expect("corpus entry").path();
        let bytes = fs::read(path).expect("corpus file");
        let _request = serde_json::from_slice::<ChatRequest>(&bytes);
    }
}
