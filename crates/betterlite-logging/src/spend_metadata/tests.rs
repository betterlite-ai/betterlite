#![cfg(test)]

use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::{SpendLogStatus, looks_like_model_name, spend_logs_id, status_for_spend_log};

fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the fixture is a JSON object")
}

#[rstest]
#[case::response_id_wins(
    r#"{"id":"resp-1"}"#,
    r#"{"standard_logging_object":{"id":"std-1"},"litellm_call_id":"call-1"}"#,
    Some("resp-1")
)]
#[case::empty_response_id_falls_through(
    r#"{"id":""}"#,
    r#"{"standard_logging_object":{"id":"std-1"},"litellm_call_id":"call-1"}"#,
    Some("std-1")
)]
#[case::nonstring_response_id_falls_through(
    r#"{"id":5}"#,
    r#"{"litellm_call_id":"call-1"}"#,
    Some("call-1")
)]
#[case::nonobject_standard_payload_is_skipped(
    r#"{"id":""}"#,
    r#"{"standard_logging_object":"std-1","litellm_call_id":"call-1"}"#,
    Some("call-1")
)]
#[case::no_candidates(r#"{"id":null}"#, r#"{}"#, None)]
fn takes_the_first_nonempty_id(
    #[case] response: &str,
    #[case] kwargs: &str,
    #[case] id: Option<&str>,
) {
    assert_eq!(
        spend_logs_id("acompletion", &object(response), &object(kwargs)).as_deref(),
        id
    );
}

#[test]
fn appends_the_batch_cost_suffix_only_for_batch_retrieval() {
    let response = object(r#"{"id":"batch-1"}"#);
    let kwargs = object("{}");
    assert_eq!(
        spend_logs_id("aretrieve_batch", &response, &kwargs).as_deref(),
        Some("batch-1_batch_cost")
    );
    assert_eq!(
        spend_logs_id("batch_retrieve", &response, &kwargs).as_deref(),
        Some("batch-1")
    );
    assert_eq!(
        spend_logs_id("acompletion", &response, &kwargs).as_deref(),
        Some("batch-1")
    );
}

#[rstest]
#[case::exact_failure(r#"{"status":"failure"}"#, SpendLogStatus::Failure)]
#[case::capitalized_is_success(r#"{"status":"Failure"}"#, SpendLogStatus::Success)]
#[case::missing_is_success(r#"{}"#, SpendLogStatus::Success)]
#[case::nonstring_is_success(r#"{"status":5}"#, SpendLogStatus::Success)]
fn reports_failure_only_for_the_exact_text(#[case] metadata: &str, #[case] status: SpendLogStatus) {
    assert_eq!(status_for_spend_log(&object(metadata)), status);
    assert_eq!(
        status.as_str(),
        if status == SpendLogStatus::Failure {
            "failure"
        } else {
            "success"
        }
    );
}

#[rstest]
#[case::plain_name("gpt-4o", true)]
#[case::mcp_prefix("MCP: gpt-4o", true)]
#[case::one_mcp_prefix_only("MCP: MCP: x", false)]
#[case::space_inside("gpt 4o", false)]
#[case::tab_inside("gpt\t4o", false)]
#[case::python_control_space("gpt\u{1c}4o", false)]
#[case::non_breaking_space("gpt\u{a0}4o", false)]
#[case::empty("", true)]
fn accepts_a_model_name_without_python_whitespace(#[case] model: &str, #[case] accepted: bool) {
    assert_eq!(looks_like_model_name(model), accepted);
}

#[test]
fn limits_a_model_name_to_256_characters() {
    assert!(looks_like_model_name(&"a".repeat(256)));
    assert!(!looks_like_model_name(&"a".repeat(257)));
    assert!(looks_like_model_name(&format!("MCP: {}", "a".repeat(256))));
    assert!(!looks_like_model_name(&format!("MCP: {}", "a".repeat(257))));
    assert!(looks_like_model_name(&"é".repeat(256)));
}
