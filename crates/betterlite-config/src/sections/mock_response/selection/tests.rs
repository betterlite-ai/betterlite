#![cfg(test)]

use super::resolve_mock;
use crate::sections::mock_response::{MockFunctionCall, MockResponse, MockToolCall};

fn text(value: &str) -> MockResponse {
    MockResponse::Text(value.to_owned())
}

fn tool_calls() -> Vec<MockToolCall> {
    vec![MockToolCall {
        id: Some(String::from("call-1")),
        kind: Some(String::from("function")),
        function: MockFunctionCall {
            name: String::from("lookup"),
            arguments: Some(String::from("{}")),
        },
    }]
}

#[test]
fn a_client_response_is_used() {
    let client = text("client");
    let plan = resolve_mock(None, Some(&client), None, None).expect("mock");
    assert_eq!(plan.response, Some(&client));
}

#[test]
fn a_client_response_wins_over_the_operator_one() {
    let operator = text("operator");
    let client = text("client");
    let plan = resolve_mock(Some(&operator), Some(&client), None, None).expect("mock");
    assert_eq!(plan.response, Some(&client));
}

#[test]
fn a_client_tool_call_is_used() {
    let calls = tool_calls();
    let plan = resolve_mock(None, None, Some(&calls), None).expect("mock");
    assert_eq!(plan.tool_calls, Some(calls.as_slice()));
}

#[test]
fn the_operator_response_is_the_fallback() {
    let operator = text("operator");
    let plan = resolve_mock(Some(&operator), None, None, None).expect("mock");
    assert_eq!(plan.response, Some(&operator));
}

#[test]
fn no_mock_field_means_no_plan() {
    assert!(resolve_mock(None, None, None, None).is_none());
}

#[test]
fn the_plan_carries_the_choice_count() {
    let client = text("client");
    let plan = resolve_mock(None, Some(&client), None, Some(3)).expect("mock");
    assert_eq!(plan.choice_count, Some(3));
    assert_eq!(plan.prompt_tokens, None);
}
