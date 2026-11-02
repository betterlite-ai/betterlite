#![cfg(test)]

use axum::http::Method;
use betterlite_billing::billable_category::BillableCategory;
use rstest::rstest;

use super::classify_billable_request;

#[rstest]
#[case::llm_chat_completions(
    Method::POST,
    "/v1/chat/completions",
    Some((BillableCategory::Llm, "/chat/completions"))
)]
#[case::mcp_route(Method::POST, "/mcp/tools/list", Some((BillableCategory::Mcp, "/mcp")))]
#[case::a2a_route(
    Method::POST,
    "/a2a/agent-1/message/send",
    Some((BillableCategory::A2a, "/a2a"))
)]
#[case::trailing_slash_normalized(
    Method::POST,
    "/v1/chat/completions/",
    Some((BillableCategory::Llm, "/chat/completions"))
)]
#[case::repeated_trailing_slashes_normalized(
    Method::POST,
    "/mcp///",
    Some((BillableCategory::Mcp, "/mcp"))
)]
#[case::unmatched_path_not_billable(Method::POST, "/health/liveliness", None)]
fn classifies_requests(
    #[case] method: Method,
    #[case] path: &str,
    #[case] expected: Option<(BillableCategory, &str)>,
) {
    assert_eq!(classify_billable_request(path, &method), expected);
}

#[test]
fn method_gate_blocks_llm_classification_for_get() {
    assert_eq!(
        classify_billable_request("/chat/completions", &Method::GET),
        None
    );
}

#[test]
fn method_gate_blocks_passthrough_llm_classification_for_get() {
    assert_eq!(
        classify_billable_request("/bedrock/anthropic.claude-3", &Method::GET),
        None
    );
}

#[test]
fn method_gate_does_not_apply_to_mcp_classification() {
    assert_eq!(
        classify_billable_request("/mcp/foo", &Method::GET),
        Some((BillableCategory::Mcp, "/mcp"))
    );
}

#[test]
fn method_gate_does_not_apply_to_a2a_classification() {
    assert_eq!(
        classify_billable_request("/a2a/agent-1/message/send", &Method::GET),
        Some((BillableCategory::A2a, "/a2a"))
    );
}
