#![cfg(test)]

use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};
use betterlite_openai::api::responses_shapes::OutputFunctionToolCall;

use crate::purview_dlp_evaluation::responses_extraction::responses_function_call_args;

fn response(output: &str) -> ResponsesApiResponse {
    serde_json::from_str(&format!(
        r#"{{"id":"resp-1","created_at":1,"output":{output}}}"#
    ))
    .unwrap()
}

#[test]
fn extracts_the_arguments_of_a_function_call_item() {
    let response = response(
        r#"[{"type":"function_call","arguments":"{\"card\":\"4111\"}","call_id":"call-1","name":"pay"}]"#,
    );
    assert_eq!(
        responses_function_call_args(&response),
        vec![r#"{"card":"4111"}"#.to_owned()]
    );
}

#[test]
fn keeps_the_arguments_exactly_as_returned() {
    let response =
        response(r#"[{"type":"function_call","arguments":"  spaced  ","call_id":"c","name":"f"}]"#);
    assert_eq!(
        responses_function_call_args(&response),
        vec!["  spaced  ".to_owned()]
    );
}

#[test]
fn skips_whitespace_only_arguments() {
    let response =
        response(r#"[{"type":"function_call","arguments":"   ","call_id":"c","name":"f"}]"#);
    assert!(responses_function_call_args(&response).is_empty());
}

#[test]
fn skips_non_string_arguments() {
    let response = response(r#"[{"type":"function_call","arguments":5,"call_id":"c","name":"f"}]"#);
    assert!(responses_function_call_args(&response).is_empty());
}

#[test]
fn scans_a_function_call_item_missing_its_typed_fields() {
    let response = response(r#"[{"type":"function_call","arguments":"{\"pin\":\"1\"}"}]"#);
    assert_eq!(
        responses_function_call_args(&response),
        vec![r#"{"pin":"1"}"#.to_owned()]
    );
}

#[test]
fn ignores_output_items_that_are_not_function_calls() {
    let response = response(
        r#"[{"type":"message","id":"m","status":"completed","role":"assistant","content":[{"type":"output_text","text":"hello"}]}]"#,
    );
    assert!(responses_function_call_args(&response).is_empty());
}

#[test]
fn returns_nothing_for_an_empty_output() {
    assert!(responses_function_call_args(&response("[]")).is_empty());
}

#[test]
fn keeps_function_calls_in_output_order() {
    let response = response(
        r#"[{"type":"function_call","arguments":"one","call_id":"c1","name":"f"},{"type":"function_call","arguments":"two","call_id":"c2","name":"g"}]"#,
    );
    assert_eq!(
        responses_function_call_args(&response),
        vec!["one".to_owned(), "two".to_owned()]
    );
}

fn with_typed_call(kind: &str, arguments: &str) -> ResponsesApiResponse {
    let call: OutputFunctionToolCall = serde_json::from_str(&format!(
        r#"{{"arguments":"{arguments}","call_id":"c","name":"f","type":"{kind}","id":null,"status":"completed"}}"#
    ))
    .unwrap();
    let mut built = response("[]");
    built.output.push(ResponseOutputItem::FunctionCall(call));
    built
}

#[test]
fn extracts_the_arguments_of_a_constructed_function_call_item() {
    assert_eq!(
        responses_function_call_args(&with_typed_call("function_call", "built")),
        vec!["built".to_owned()]
    );
}

#[test]
fn skips_a_constructed_call_of_another_type() {
    assert!(responses_function_call_args(&with_typed_call("custom_tool_call", "built")).is_empty());
}

#[test]
fn skips_an_untyped_item_of_another_type() {
    let response = response(r#"[{"type":"custom_call","arguments":"secret"}]"#);
    assert!(responses_function_call_args(&response).is_empty());
}
