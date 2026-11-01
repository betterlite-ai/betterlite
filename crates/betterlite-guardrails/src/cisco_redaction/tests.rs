#![cfg(test)]

use rstest::rstest;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_redaction::chat_input::redact_chat_input;
use crate::cisco_redaction::chat_output::{ListField, redact_list_field, redact_model_response};
use crate::cisco_redaction::mcp::{apply_redaction_mcp_input, apply_redaction_mcp_output};
use crate::cisco_verdict;

fn obj(json: &str) -> Fields {
    let Json::Object(fields) = Json::parse(json) else {
        panic!("expected a JSON object, got: {json}");
    };
    fields
}

fn objects(json: &str) -> Vec<Fields> {
    let Json::List(items) = Json::parse(json) else {
        panic!("expected a JSON array, got: {json}");
    };
    items.iter().map(|item| obj(item.get())).collect()
}

fn text_field(fields: &Fields, key: &str) -> String {
    match Json::field(fields, key) {
        Json::Text(text) => text,
        other
        @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            panic!("expected {key} to be a string, got {other:?}")
        }
    }
}

#[rstest]
#[case::mcp_arguments_key("mcp_arguments", r#"{"mcp_arguments": {"question": "secret"}}"#)]
#[case::arguments_key("arguments", r#"{"arguments": {"question": "secret"}}"#)]
#[case::params_arguments_key("params", r#"{"params": {"arguments": {"question": "secret"}}}"#)]
fn apply_redaction_mcp_input_rewrites_single_string_argument(
    #[case] candidate_key: &str,
    #[case] request_json: &str,
) {
    let mut request_data = obj(request_json);
    let applied = apply_redaction_mcp_input(&mut request_data, Some("redacted"), None);
    assert!(applied);
    let rewritten = match candidate_key {
        "params" => {
            let Json::Object(params) = Json::field(&request_data, "params") else {
                panic!("params missing");
            };
            let Json::Object(arguments) = Json::field(&params, "arguments") else {
                panic!("params.arguments missing");
            };
            arguments
        }
        key => {
            let Json::Object(arguments) = Json::field(&request_data, key) else {
                panic!("{key} missing");
            };
            arguments
        }
    };
    assert_eq!(text_field(&rewritten, "question"), "redacted");
}

#[test]
fn apply_redaction_mcp_input_sets_sanitized_mcp_arguments_everywhere_present() {
    let mut request_data =
        obj(r#"{"params": {"arguments": {}}, "arguments": {"question": "secret"}}"#);
    let sanitized = obj(r#"{"question": "redacted"}"#);
    let applied = apply_redaction_mcp_input(&mut request_data, None, Some(&sanitized));
    assert!(applied);
    assert_eq!(
        text_field(
            &match Json::field(&request_data, "mcp_arguments") {
                Json::Object(fields) => fields,
                other @ (Json::Null
                | Json::Bool(_)
                | Json::Number(_)
                | Json::Text(_)
                | Json::List(_)) => {
                    panic!("expected object, got {other:?}")
                }
            },
            "question"
        ),
        "redacted"
    );
    let Json::Object(params) = Json::field(&request_data, "params") else {
        panic!("params missing");
    };
    let Json::Object(params_arguments) = Json::field(&params, "arguments") else {
        panic!("params.arguments missing");
    };
    assert_eq!(text_field(&params_arguments, "question"), "redacted");
    let Json::Object(arguments) = Json::field(&request_data, "arguments") else {
        panic!("arguments missing");
    };
    assert_eq!(text_field(&arguments, "question"), "redacted");
}

#[test]
fn apply_redaction_mcp_input_skips_multi_string_argument_objects() {
    let mut request_data = obj(r#"{"arguments": {"a": "x", "b": "y"}}"#);
    let applied = apply_redaction_mcp_input(&mut request_data, Some("redacted"), None);
    assert!(!applied);
}

#[test]
fn apply_redaction_mcp_input_returns_false_without_any_surface() {
    let mut request_data = obj("{}");
    assert!(!apply_redaction_mcp_input(
        &mut request_data,
        Some("redacted"),
        None
    ));
    assert!(!apply_redaction_mcp_input(&mut request_data, None, None));
}

#[test]
fn apply_redaction_chat_input_strips_synthetic_tool_definition_message() {
    let mut request_data = obj(r#"{"tools": [{"name": "lookup"}], "messages": []}"#);
    let sanitized_messages = objects(
        r#"[{"role": "user", "content": "hi"}, {"role": "system", "content": "tool definitions"}]"#,
    );
    let applied = redact_chat_input(&mut request_data, None, Some(&sanitized_messages));
    assert!(applied);
    let Json::List(messages) = Json::field(&request_data, "messages") else {
        panic!("messages missing");
    };
    assert_eq!(messages.len(), 1);
}

#[test]
fn apply_redaction_chat_input_sets_instructions_from_system_message_and_stops() {
    let mut request_data = obj(r#"{"instructions": "old", "input": "hi"}"#);
    let sanitized_messages = objects(r#"[{"role": "system", "content": "new instructions"}]"#);
    let applied = redact_chat_input(&mut request_data, None, Some(&sanitized_messages));
    assert!(applied);
    assert_eq!(
        text_field(&request_data, "instructions"),
        "new instructions"
    );
    assert!(
        !request_data.contains_key("input")
            || matches!(
                Json::field(&request_data, "input"),
                Json::Text(text) if text == "hi"
            )
    );
}

#[test]
fn apply_redaction_chat_input_falls_back_to_sanitized_text_for_instructions() {
    let mut request_data = obj(r#"{"instructions": "old"}"#);
    let applied = redact_chat_input(&mut request_data, Some("new text"), None);
    assert!(applied);
    assert_eq!(text_field(&request_data, "instructions"), "new text");
}

#[test]
fn apply_redaction_chat_input_rewrites_messages_in_place() {
    let mut request_data = obj(r#"{"messages": [{"role": "user", "content": "old"}]}"#);
    let sanitized_messages = objects(r#"[{"role": "user", "content": "new"}]"#);
    let applied = redact_chat_input(&mut request_data, None, Some(&sanitized_messages));
    assert!(applied);
    let Json::List(messages) = Json::field(&request_data, "messages") else {
        panic!("messages missing");
    };
    let rewritten = obj(messages[0].get());
    assert_eq!(text_field(&rewritten, "content"), "new");
}

#[test]
fn apply_redaction_chat_input_converts_messages_to_responses_input() {
    let mut request_data = obj(r#"{"input": "old"}"#);
    let sanitized_messages = objects(r#"[{"role": "user", "content": "new"}]"#);
    let applied = redact_chat_input(&mut request_data, None, Some(&sanitized_messages));
    assert!(applied);
    assert!(matches!(Json::field(&request_data, "input"), Json::List(_)));
}

#[test]
fn apply_redaction_chat_input_clears_tool_arguments_and_rewrites_last_user_message() {
    let mut request_data = obj(r#"{"messages": [
            {"role": "user", "content": "old"},
            {"role": "assistant", "content": "", "tool_calls": [{"function": {"name": "f", "arguments": "{\"x\":1}"}}]}
        ]}"#);
    let applied = redact_chat_input(&mut request_data, Some("redacted"), None);
    assert!(applied);
    let Json::List(messages) = Json::field(&request_data, "messages") else {
        panic!("messages missing");
    };
    let assistant = obj(messages[1].get());
    let Json::List(tool_calls) = Json::field(&assistant, "tool_calls") else {
        panic!("tool_calls missing");
    };
    let call = obj(tool_calls[0].get());
    let Json::Object(function) = Json::field(&call, "function") else {
        panic!("function missing");
    };
    assert_eq!(text_field(&function, "arguments"), "{}");
    let user = obj(messages[0].get());
    assert_eq!(text_field(&user, "content"), "redacted");
}

#[test]
fn apply_redaction_chat_input_returns_false_with_no_surface() {
    let mut request_data = obj("{}");
    assert!(!redact_chat_input(&mut request_data, None, None));
}

#[test]
fn redact_model_response_consumes_one_message_per_choice() {
    let mut response = obj(r#"{"choices": [
            {"message": {"content": "old", "reasoning_content": "secret thought"}}
        ]}"#);
    let sanitized_messages = objects(r#"[{"role": "assistant", "content": "redacted"}]"#);
    let applied = redact_model_response(Some(&mut response), None, Some(&sanitized_messages));
    assert!(applied);
    let Json::List(choices) = Json::field(&response, "choices") else {
        panic!("choices missing");
    };
    let choice = obj(choices[0].get());
    let Json::Object(message) = Json::field(&choice, "message") else {
        panic!("message missing");
    };
    assert_eq!(text_field(&message, "content"), "redacted");
    assert!(!message.contains_key("reasoning_content"));
}

#[test]
fn redact_model_response_uses_sanitized_text_fallback() {
    let mut response = obj(r#"{"choices": [{"message": {"content": "old"}}]}"#);
    let applied = redact_model_response(Some(&mut response), Some("redacted"), None);
    assert!(applied);
    let Json::List(choices) = Json::field(&response, "choices") else {
        panic!("choices missing");
    };
    let choice = obj(choices[0].get());
    let Json::Object(message) = Json::field(&choice, "message") else {
        panic!("message missing");
    };
    assert_eq!(text_field(&message, "content"), "redacted");
}

#[test]
fn redact_model_response_returns_false_without_choices() {
    let mut response = obj("{}");
    assert!(!redact_model_response(
        Some(&mut response),
        Some("redacted"),
        None
    ));
    assert!(!redact_model_response(None, Some("redacted"), None));
}

#[test]
fn redact_text_completion_rewrites_choice_text() {
    let mut response = obj(r#"{"choices": [{"text": "old"}]}"#);
    let applied = redact_list_field(
        Some(&mut response),
        Some("redacted"),
        None,
        ListField::TextCompletion,
    );
    assert!(applied);
    let Json::List(choices) = Json::field(&response, "choices") else {
        panic!("choices missing");
    };
    let choice = obj(choices[0].get());
    assert_eq!(text_field(&choice, "text"), "redacted");
}

#[test]
fn redact_text_completion_falls_back_to_message_content() {
    let mut response = obj(r#"{"choices": [{"text": "old"}]}"#);
    let sanitized_messages = objects(r#"[{"role": "assistant", "content": "from message"}]"#);
    let applied = redact_list_field(
        Some(&mut response),
        None,
        Some(&sanitized_messages),
        ListField::TextCompletion,
    );
    assert!(applied);
    let Json::List(choices) = Json::field(&response, "choices") else {
        panic!("choices missing");
    };
    let choice = obj(choices[0].get());
    assert_eq!(text_field(&choice, "text"), "from message");
}

#[test]
fn redact_text_completion_returns_false_without_replacement() {
    let mut response = obj(r#"{"choices": [{"text": "old"}]}"#);
    assert!(!redact_list_field(
        Some(&mut response),
        None,
        None,
        ListField::TextCompletion
    ));
    assert!(!redact_list_field(
        None,
        Some("redacted"),
        None,
        ListField::TextCompletion
    ));
}

#[test]
fn redact_responses_output_rewrites_text_parts_and_clears_arguments() {
    let mut response = obj(r#"{"output": [
            {"content": [{"type": "text", "text": "old"}]},
            {"arguments": "{\"x\":1}"}
        ]}"#);
    let applied = redact_list_field(
        Some(&mut response),
        Some("redacted"),
        None,
        ListField::ResponsesOutput,
    );
    assert!(applied);
    let Json::List(output) = Json::field(&response, "output") else {
        panic!("output missing");
    };
    let first = obj(output[0].get());
    let Json::List(content) = Json::field(&first, "content") else {
        panic!("content missing");
    };
    let part = obj(content[0].get());
    assert_eq!(text_field(&part, "text"), "redacted");
    let second = obj(output[1].get());
    assert_eq!(text_field(&second, "arguments"), "{}");
}

#[test]
fn redact_responses_output_joins_sanitized_message_content() {
    let mut response = obj(r#"{"output": [{"content": [{"type": "text", "text": "old"}]}]}"#);
    let sanitized_messages = objects(
        r#"[{"role": "assistant", "content": "one"}, {"role": "assistant", "content": "two"}]"#,
    );
    let applied = redact_list_field(
        Some(&mut response),
        None,
        Some(&sanitized_messages),
        ListField::ResponsesOutput,
    );
    assert!(applied);
    let Json::List(output) = Json::field(&response, "output") else {
        panic!("output missing");
    };
    let item = obj(output[0].get());
    let Json::List(content) = Json::field(&item, "content") else {
        panic!("content missing");
    };
    let part = obj(content[0].get());
    assert_eq!(text_field(&part, "text"), "one two");
}

#[test]
fn redact_responses_output_returns_false_without_replacement() {
    let mut response = obj(r#"{"output": [{"content": []}]}"#);
    assert!(!redact_list_field(
        Some(&mut response),
        None,
        None,
        ListField::ResponsesOutput
    ));
    assert!(!redact_list_field(
        None,
        Some("redacted"),
        None,
        ListField::ResponsesOutput
    ));
}

#[test]
fn apply_redaction_mcp_output_delegates_to_cisco_mcp_redact() {
    let response = Json::parse(r#"{"content": [{"type": "text", "text": "old"}]}"#);
    let update =
        apply_redaction_mcp_output(Some(&response), Some("redacted")).expect("delegated update");
    assert!(update.replaced);
}

#[test]
fn apply_redaction_mcp_output_returns_none_without_response_or_text() {
    let response = Json::parse(r#"{"content": []}"#);
    assert!(apply_redaction_mcp_output(None, Some("redacted")).is_none());
    assert!(apply_redaction_mcp_output(Some(&response), None).is_none());
}

#[test]
fn extract_masked_entity_count_rules_reuses_cisco_verdict() {
    let rules =
        Json::parse(r#"[{"entity_types": ["EMAIL", "PHONE"]}, {"entity_types": ["EMAIL"]}]"#);
    let counts = cisco_verdict::extract_masked_entity_count(&rules).expect("counts present");
    assert_eq!(counts.get("EMAIL").copied(), Some(2));
    assert_eq!(counts.get("PHONE").copied(), Some(1));
}
