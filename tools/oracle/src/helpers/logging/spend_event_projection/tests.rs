#![cfg(test)]

use serde_json::{Value, json};

use super::Op;
use crate::helpers::Helper as _;

fn build_input(kwargs: &Value, response: &Value) -> Value {
    json!({
        "function": "build",
        "kwargs": kwargs,
        "litellm_params": {},
        "metadata": null,
        "litellm_metadata": {"user_api_key_budget_reservation": {"id": "res-1"}},
        "typed_auth": {"metadata": null, "litellm_metadata": null},
        "response": response,
        "start_time": "2026-01-01T00:00:00Z",
        "end_time": "2026-01-01T00:00:01Z",
        "store_bodies": false,
        "cache": "off",
    })
}

#[test]
fn a_build_names_the_response_tools_with_the_shared_extractor() {
    let response = json!({
        "kind": "mapping",
        "fields": {"id": "r-1", "choices": [{"message": {"tool_calls": [
            {"id": "t1", "function": {"name": " lookup ", "arguments": "{}"}}]}}]},
    });
    let output = Op::decode_and_run(&build_input(&json!({"model": "gpt-4"}), &response)).unwrap();
    assert_eq!(output["offloadable"], true);
    assert_eq!(output["newline_terminated"], true);
    let call = &output["event"]["response"]["choices"][0]["message"]["tool_calls"][0];
    assert_eq!(call["function"]["name"], "lookup");
    assert_eq!(
        output["event"]["litellm_params"]["preset_cache_key"],
        "Cache OFF"
    );
    let copied = &output["event"]["litellm_params"]["litellm_metadata"];
    assert_eq!(copied["user_api_key_budget_reservation"]["id"], "res-1");
    assert_eq!(output["event"]["litellm_params"]["metadata"], Value::Null);
}

#[test]
fn a_kwargs_projection_failure_is_a_build_error_value() {
    let input = build_input(&json!({"stream": "yes"}), &json!({"kind": "absent"}));
    let output = Op::decode_and_run(&input).unwrap();
    assert_eq!(output, json!({"build_error": true}));
}

#[test]
fn a_rejected_line_is_a_decode_error_value() {
    let line = r#"{"version": 2}"#;
    let decoded = Op::decode_and_run(&json!({"function": "decode", "line": line})).unwrap();
    let restored = Op::decode_and_run(&json!({"function": "restore", "line": line})).unwrap();
    assert_eq!(decoded, json!({"decode_error": true}));
    assert_eq!(restored, json!({"decode_error": true}));
}

#[rstest::rstest]
#[case::absent("absent", true)]
#[case::model("model", true)]
#[case::mapping("mapping", true)]
#[case::batch("batch", false)]
#[case::interactions("interactions", false)]
fn only_batch_and_interactions_stay_in_process(#[case] kind: &str, #[case] expected: bool) {
    let output = Op::decode_and_run(&json!({"function": "offloadable", "kind": kind})).unwrap();
    assert_eq!(output["offloadable"], expected);
}

#[test]
fn an_unknown_input_field_is_refused() {
    let input = json!({"function": "decode", "line": "{}", "extra": 1_u8});
    assert!(Op::check(&input).is_err());
}
