#![cfg(test)]

use rstest::rstest;

use super::count_web_search_calls;
use crate::decisions::Json;
use crate::tool_signals::response::{
    ChatView, OtherView, OutputItem, RawView, ResponsesView, ToolBillingResponse,
};

fn mapping(source: &str) -> OutputItem {
    OutputItem::Mapping(serde_json::from_str(source).expect("the item is a mapping"))
}

fn attribute(source: &str) -> OutputItem {
    OutputItem::Attribute(serde_json::from_str(source).expect("the item is a mapping"))
}

fn web_search_item() -> OutputItem {
    mapping(r#"{"type":"web_search_call"}"#)
}

fn responses_with(output: Vec<OutputItem>, tool_usage: Option<&str>) -> ToolBillingResponse {
    ToolBillingResponse::Responses(ResponsesView {
        output,
        tool_usage: tool_usage
            .map(|source| serde_json::from_str(source).expect("the tool usage is valid JSON")),
        usage: None,
    })
}

#[test]
fn non_responses_objects_bill_a_single_call() {
    let chat = ToolBillingResponse::Chat(ChatView {
        choices: Vec::new(),
        usage: None,
    });
    let other = ToolBillingResponse::Other(OtherView { usage: None });
    let raw = ToolBillingResponse::Raw(RawView {
        body: Json::Null,
        usage: None,
    });
    assert_eq!(count_web_search_calls(&chat), Ok(1));
    assert_eq!(count_web_search_calls(&other), Ok(1));
    assert_eq!(count_web_search_calls(&raw), Ok(1));
}

#[test]
fn a_reported_zero_beats_the_output_item_count() {
    let response = responses_with(
        vec![web_search_item(), web_search_item(), web_search_item()],
        Some(r#"{"web_search":{"num_requests":0}}"#),
    );
    assert_eq!(count_web_search_calls(&response), Ok(0));
}

#[test]
fn a_reported_count_beats_the_output_item_count() {
    let response = responses_with(
        vec![web_search_item(), web_search_item()],
        Some(r#"{"web_search":{"num_requests":7}}"#),
    );
    assert_eq!(count_web_search_calls(&response), Ok(7));
}

#[test]
fn untyped_output_items_are_counted() {
    let response = responses_with(
        vec![
            web_search_item(),
            attribute(r#"{"type":"web_search_call"}"#),
            mapping(r#"{"type":"message"}"#),
        ],
        None,
    );
    assert_eq!(count_web_search_calls(&response), Ok(2));
}

#[test]
fn a_missing_tool_usage_counts_the_output_items() {
    let response = responses_with(vec![web_search_item(), web_search_item()], None);
    assert_eq!(count_web_search_calls(&response), Ok(2));
}

#[test]
fn an_empty_output_floors_at_one_call() {
    assert_eq!(
        count_web_search_calls(&responses_with(Vec::new(), None)),
        Ok(1)
    );
}

#[rstest]
#[case::negative(r#"{"web_search":{"num_requests":-1}}"#)]
#[case::missing_count(r#"{"web_search":{}}"#)]
#[case::text_count(r#"{"web_search":{"num_requests":"many"}}"#)]
#[case::fractional_count(r#"{"web_search":{"num_requests":2.5}}"#)]
#[case::list_search(r#"{"web_search":[]}"#)]
#[case::not_a_mapping("5")]
#[case::null_tool_usage("null")]
fn invalid_tool_usage_falls_back_to_counting_items(#[case] tool_usage: &str) {
    let response = responses_with(vec![web_search_item(), web_search_item()], Some(tool_usage));
    assert_eq!(count_web_search_calls(&response), Ok(2));
}

#[rstest]
#[case::integral_float(r#"{"web_search":{"num_requests":3.0}}"#)]
#[case::numeric_string(r#"{"web_search":{"num_requests":"3"}}"#)]
#[case::boolean(r#"{"web_search":{"num_requests":true}}"#)]
fn pydantic_lax_counts_are_reported(#[case] tool_usage: &str) {
    let response = responses_with(Vec::new(), Some(tool_usage));
    let expected = if tool_usage.contains("true") { 1 } else { 3 };
    assert_eq!(count_web_search_calls(&response), Ok(expected));
}

#[test]
fn a_reported_false_is_a_zero_count() {
    let response = responses_with(Vec::new(), Some(r#"{"web_search":{"num_requests":false}}"#));
    assert_eq!(count_web_search_calls(&response), Ok(0));
}

#[rstest]
#[case::wrong_type(r#"{"type":"Web_Search_Call"}"#)]
#[case::numeric_type(r#"{"type":5}"#)]
#[case::missing_type(r#"{"status":"completed"}"#)]
fn only_a_string_output_type_is_a_web_search_call(#[case] item: &str) {
    let response = responses_with(vec![mapping(item)], None);
    assert_eq!(count_web_search_calls(&response), Ok(1));
}

#[test]
fn an_attribute_item_without_a_string_type_is_not_counted() {
    let response = responses_with(vec![attribute(r#"{"type":null}"#)], None);
    assert_eq!(count_web_search_calls(&response), Ok(1));
}

#[test]
fn a_raw_mapping_with_an_output_key_stays_one_call() {
    let response = ToolBillingResponse::Raw(RawView {
        body: serde_json::from_str(r#"{"output":[{"type":"web_search_call"}]}"#)
            .expect("the body is valid JSON"),
        usage: None,
    });
    assert_eq!(count_web_search_calls(&response), Ok(1));
}
