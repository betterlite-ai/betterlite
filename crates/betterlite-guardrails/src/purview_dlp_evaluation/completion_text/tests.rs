#![cfg(test)]

use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_prompts::arguments::json::Object;

use crate::purview_dlp_evaluation::completion_text::{
    CompletionResponse, completion_response_text_parts,
};

fn body(raw: &str) -> Object {
    serde_json::from_str(raw).unwrap()
}

fn responses(raw: &str) -> ResponsesApiResponse {
    serde_json::from_str(raw).unwrap()
}

#[test]
fn collects_the_text_of_every_text_completion_choice() {
    let response =
        body(r#"{"object":"text_completion","choices":[{"text":"hello"},{"text":"world"}]}"#);
    assert_eq!(
        completion_response_text_parts(&CompletionResponse::TextCompletion(&response)),
        vec!["hello".to_owned(), "world".to_owned()]
    );
}

#[test]
fn skips_blank_text_completion_choices() {
    let response = body(r#"{"choices":[{"text":"  "},{"text":"kept"}]}"#);
    assert_eq!(
        completion_response_text_parts(&CompletionResponse::TextCompletion(&response)),
        vec!["kept".to_owned()]
    );
}

#[test]
fn keeps_text_completion_text_exactly_as_returned() {
    let response = body(r#"{"choices":[{"text":"  padded  "}]}"#);
    assert_eq!(
        completion_response_text_parts(&CompletionResponse::TextCompletion(&response)),
        vec!["  padded  ".to_owned()]
    );
}

#[test]
fn returns_nothing_for_a_text_completion_without_choices() {
    let response = body(r#"{"object":"text_completion"}"#);
    assert!(
        completion_response_text_parts(&CompletionResponse::TextCompletion(&response)).is_empty()
    );
}

#[test]
fn collects_message_content_of_every_chat_choice() {
    let response = body(
        r#"{"choices":[{"message":{"role":"assistant","content":"hello"}},{"message":{"role":"assistant","content":"world"}}]}"#,
    );
    assert_eq!(
        completion_response_text_parts(&CompletionResponse::Model(&response)),
        vec!["hello".to_owned(), "world".to_owned()]
    );
}

#[test]
fn skips_blank_and_missing_message_content() {
    let response = body(
        r#"{"choices":[{"message":{"content":"   "}},{"message":{"role":"assistant"}},{"message":{"content":"kept"}}]}"#,
    );
    assert_eq!(
        completion_response_text_parts(&CompletionResponse::Model(&response)),
        vec!["kept".to_owned()]
    );
}

#[test]
fn skips_choices_without_a_message() {
    let response = body(r#"{"choices":[{"message":null},{"message":{"content":"kept"}}]}"#);
    assert_eq!(
        completion_response_text_parts(&CompletionResponse::Model(&response)),
        vec!["kept".to_owned()]
    );
}

#[test]
fn includes_the_model_generated_tool_call_arguments() {
    let response = body(
        r#"{"choices":[{"message":{"role":"assistant","content":"checking","tool_calls":[{"id":"call-1","type":"function","function":{"name":"pay","arguments":"{\"card\":\"4111\"}"}}]}}]}"#,
    );
    assert_eq!(
        completion_response_text_parts(&CompletionResponse::Model(&response)),
        vec!["checking".to_owned(), r#"{"card":"4111"}"#.to_owned()]
    );
}

#[test]
fn includes_the_legacy_function_call_arguments() {
    let response = body(
        r#"{"choices":[{"message":{"role":"assistant","function_call":{"name":"pay","arguments":"{\"card\":\"4111\"}"}}}]}"#,
    );
    assert_eq!(
        completion_response_text_parts(&CompletionResponse::Model(&response)),
        vec![r#"{"card":"4111"}"#.to_owned()]
    );
}

#[test]
fn collects_the_responses_output_text() {
    let response = responses(
        r#"{"id":"resp-1","created_at":1,"output":[{"type":"message","id":"m","status":"completed","role":"assistant","content":[{"type":"output_text","annotations":[],"text":"hello "},{"type":"output_text","annotations":[],"text":"there"}]}]}"#,
    );
    assert_eq!(
        completion_response_text_parts(&CompletionResponse::Responses(&response)),
        vec!["hello there".to_owned()]
    );
}

#[test]
fn collects_the_responses_function_call_arguments_after_the_text() {
    let response = responses(
        r#"{"id":"resp-1","created_at":1,"output":[{"type":"message","id":"m","status":"completed","role":"assistant","content":[{"type":"output_text","annotations":[],"text":"hello"}]},{"type":"function_call","arguments":"{\"pin\":\"1\"}","call_id":"c","name":"pay"}]}"#,
    );
    assert_eq!(
        completion_response_text_parts(&CompletionResponse::Responses(&response)),
        vec!["hello".to_owned(), r#"{"pin":"1"}"#.to_owned()]
    );
}

#[test]
fn returns_nothing_for_a_responses_answer_without_scannable_parts() {
    let response = responses(r#"{"id":"resp-1","created_at":1,"output":[]}"#);
    assert!(completion_response_text_parts(&CompletionResponse::Responses(&response)).is_empty());
}
