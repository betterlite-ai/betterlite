#![cfg(test)]

use super::{detect_first_expected_role, get_completion_messages};
use rstest::rstest;
use serde_json::{Value, json};

#[rstest]
#[case::empty_first_role("role", json!([]), json!("user"))]
#[case::system_then_user("role", json!([{"role":"system"},{"role":"user"}]), json!("assistant"))]
#[case::assistant_first("role", json!([{"role":"assistant"}]), json!("user"))]
#[case::only_system("role", json!([{"role":"system"}]), Value::Null)]
#[case::insert_before_and_after_assistant("complete", json!([{"role":"assistant","content":"a"}]), json!([{"role":"user","content":"Please continue."},{"role":"assistant","content":"a"},{"role":"user","content":"Please continue."}]))]
#[case::disabled_returns_copy("disabled", json!([{"role":"assistant"}]), json!([{"role":"assistant"}]))]
#[case::empty_conversation("complete", json!([]), json!([]))]
#[case::tool_call_assistant_does_not_count("complete", json!([{"role":"user"},{"role":"assistant","tool_calls":[{"id":"c"}]},{"role":"tool"},{"role":"user"}]), json!([{"role":"user"},{"role":"assistant","tool_calls":[{"id":"c"}]},{"role":"tool"},{"role":"assistant","content":"Please continue."},{"role":"user"}]))]
#[case::preserves_fields("complete", json!([{"role":"assistant","content":"x","signature":"keep"}]), json!([{"role":"user","content":"Please continue."},{"role":"assistant","content":"x","signature":"keep"},{"role":"user","content":"Please continue."}]))]
fn message_normalization_message_tests_behavior(
    #[case] operation: &str,
    #[case] input: Value,
    #[case] expected: Value,
) {
    let messages = input.as_array().cloned().unwrap_or_default();
    let original = messages.clone();
    let actual = match operation {
        "role" => json!(detect_first_expected_role(&messages)),
        "complete" => json!(get_completion_messages(&messages, None, None, true)),
        "disabled" => json!(get_completion_messages(&messages, None, None, false)),
        _ => Value::Null,
    };
    assert_eq!(actual, expected);
    assert_eq!(messages, original);
}
