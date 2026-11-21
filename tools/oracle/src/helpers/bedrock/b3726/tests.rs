#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_bedrock::guardrail_hooks::chunks::StreamChunk;
use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;
use serde_json::json;

#[rstest]
fn replays_the_bedrock_guardrail_hook_corpus(
    #[base_dir = "../../.."]
    #[files("cases/bedrock_guardrail_hooks/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(&path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    assert!(
        matches!(
            run_case(&[crate::GROUP], &case, false).unwrap(),
            HelperRun::Match
        ),
        "{} does not match LiteLLM",
        path.display()
    );
}

#[test]
fn a_text_completion_stream_does_not_assemble_to_a_chat_response() {
    let chunks = vec![StreamChunk::Json(
        serde_json::from_value(json!({
            "id": "cmpl-1",
            "model": "gpt-3.5-turbo-instruct",
            "choices": [{"index": 0, "text": "Hel", "finish_reason": null}],
        }))
        .unwrap(),
    )];
    assert_eq!(super::assembly::assemble(&chunks, false), None);
}
