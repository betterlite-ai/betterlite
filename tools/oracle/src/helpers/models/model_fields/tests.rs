#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use rstest::rstest;

#[rstest]
#[case::limits(
    r#"{"source":"{\"litellm_provider\":\"openai\",\"max_input_tokens\":128000,\"rpm\":10000}"}"#,
    "/max_input_tokens",
    "128000"
)]
#[case::omitted_flag_is_absent(
    r#"{"source":"{\"litellm_provider\":\"openai\",\"supports_vision\":false}"}"#,
    "/supports_audio_input",
    "null"
)]
#[case::explicit_false_flag_stays_present(
    r#"{"source":"{\"litellm_provider\":\"openai\",\"supports_vision\":false}"}"#,
    "/supports_vision",
    "false"
)]
fn canonical_projection(#[case] source: &str, #[case] path: &str, #[case] expected: &str) {
    let input: Input = serde_json::from_str(source).unwrap();
    let output = Op::run(&input).unwrap();
    assert_eq!(output.pointer(path).map(ToString::to_string), {
        if expected == "null" {
            None
        } else {
            Some(expected.to_owned())
        }
    });
}

#[test]
fn reports_an_unknown_fields_complete_path() {
    let input: Input = serde_json::from_str(
        r#"{"source":"{\"litellm_provider\":\"openai\",\"totally_new_field\":true,\"metadata\":{\"brand_new_note\":\"x\"}}"}"#,
    )
    .unwrap();
    let output = Op::run(&input).unwrap();
    let unknown_fields = output.pointer("/unknown_fields").unwrap().as_array().unwrap();
    let paths: Vec<&str> = unknown_fields.iter().filter_map(|value| value.as_str()).collect();
    assert_eq!(paths, ["metadata.brand_new_note", "totally_new_field"]);
}

#[test]
fn rejects_the_documentation_sample_entry() {
    let input: Input = serde_json::from_str(
        r#"{"source":"{\"litellm_provider\":\"one of https://docs.litellm.ai/docs/providers\",\"mode\":\"one of: chat, embedding\"}"}"#,
    )
    .unwrap();
    let error = Op::run(&input).unwrap_err();
    assert_eq!(error.class, "ModelFieldError");
}

#[test]
fn types_a_fallback_generalization_rule_without_executing_it() {
    let input: Input = serde_json::from_str(
        r#"{"kind":"fallback_generalizations","source":"{\"rules\":[{\"name\":\"bedrock-claude-ids\",\"pattern\":\"bedrock/.*claude.*\",\"model_info\":{\"litellm_provider\":\"bedrock\"}}]}"}"#,
    )
    .unwrap();
    let output = Op::run(&input).unwrap();
    assert_eq!(
        output.pointer("/rules/0/model_info/litellm_provider"),
        Some(&serde_json::Value::String("bedrock".to_owned()))
    );
}
