#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;

#[rstest::rstest]
#[case::null("null")]
#[case::boolean("true")]
#[case::integer("4")]
#[case::string(r#""total_input_tokens""#)]
#[case::array("[]")]
fn non_mapping_usage_is_not_recognized(#[case] source: &str) {
    let raw = serde_json::from_str(source).unwrap();
    let output = super::interactions_result(&raw).unwrap();
    assert_eq!(
        output,
        serde_json::json!({"recognized": false, "usage": null})
    );
}

#[test]
fn transcription_duration_projects_to_null() {
    let input: Input = serde_json::from_str(
        r#"{"transcription":[{"type":"duration","seconds":1.5}],"interactions":[]}"#,
    )
    .unwrap();
    let output = Op::run(&input).unwrap();
    assert_eq!(output["transcription"][0], serde_json::Value::Null);
}

#[test]
fn transcription_tokens_copies_counts() {
    let input: Input = serde_json::from_str(
        r#"{"transcription":[{"type":"tokens","input_tokens":10,"output_tokens":3,
        "total_tokens":13,"input_token_details":{"audio_tokens":7,"text_tokens":3}}],
        "interactions":[]}"#,
    )
    .unwrap();
    let output = Op::run(&input).unwrap();
    let usage = &output["transcription"][0];
    assert_eq!(usage["prompt_tokens"], 10_i64);
    assert_eq!(usage["prompt_details"]["audio_tokens"], 7_i64);
}

#[test]
fn interactions_recognition_flag_is_reported() {
    let input: Input = serde_json::from_str(
        r#"{"transcription":[],"interactions":[{"prompt_tokens":4,"total_input_tokens":4},
        {"total_input_tokens":4,"total_output_tokens":2}]}"#,
    )
    .unwrap();
    let output = Op::run(&input).unwrap();
    assert_eq!(output["interactions"][0]["recognized"], false);
    assert_eq!(output["interactions"][1]["recognized"], true);
    assert_eq!(output["interactions"][1]["usage"]["total_tokens"], 6_i64);
}

#[test]
fn malformed_modality_entry_is_a_typed_error() {
    let input: Input = serde_json::from_str(
        r#"{"transcription":[],"interactions":[{"total_input_tokens":0,"total_output_tokens":0,
        "input_tokens_by_modality":["not an object"]}]}"#,
    )
    .unwrap();
    let error = Op::run(&input).unwrap_err();
    assert_eq!(error.class, "ModalityEntryNotObject");
}
