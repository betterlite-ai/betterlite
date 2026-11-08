#![cfg(test)]

use std::fs;
use std::fs::read_to_string;
use std::path::PathBuf;
use std::sync::Arc;

use betterlite_config::sections::mock_response::MockResponse;
use betterlite_config::sections::mock_response::selection::MockPermission;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Value;
use tracing::subscriber::with_default;

use super::{
    ChatRequest, MessageContent, RequestError, Role, Stop, ToolChoice, choice_finish_reason,
    client_mock, mock_model,
};

#[test]
fn parses_typed_fields_and_keeps_unknown_parameters_for_forwarding() {
    let body = br#"{"messages":[{"role":"user","content":[{"type":"text","text":"hi"}],"cache_control":{"type":"ephemeral"}}],"stop":"\n","tool_choice":"auto","n":2,"temperature":1,"vendor_flag":true}"#;
    let request = ChatRequest::parse(body).unwrap();
    assert_eq!(request.model, None);
    assert_eq!(request.choice_count, Some(2));
    assert_eq!(request.stop, Some(Stop::One("\n".to_owned())));
    assert_eq!(
        request.tool_choice,
        Some(ToolChoice::Mode("auto".to_owned()))
    );
    let message = request.messages.first().unwrap();
    assert_eq!(message.role, Role::User);
    assert!(matches!(message.content, Some(MessageContent::Parts(_))));
    assert_eq!(
        message.passthrough.keys().collect::<Vec<_>>(),
        ["cache_control"]
    );
    assert_eq!(
        request.passthrough.keys().collect::<Vec<_>>(),
        ["vendor_flag"]
    );
    let written = serde_json::to_value(&request).unwrap();
    assert_eq!(written.get("temperature").unwrap(), 1_u64);
    assert_eq!(written.get("n").unwrap(), 2_u64);
}

#[rstest]
#[case::api_base("api_base")]
#[case::fallbacks("fallbacks")]
#[case::mock_delay("mock_delay")]
#[case::mock_timeout("mock_timeout")]
#[case::extra_headers("extra_headers")]
fn refuses_litellm_request_keys_it_does_not_act_on(#[case] name: &str) {
    let body = format!(r#"{{"model":"m","messages":[],"{name}":"x"}}"#);
    let error = ChatRequest::parse(body.as_bytes()).unwrap_err();
    assert!(matches!(error, RequestError::Unsupported { name: refused } if refused == name));
}

fn allowed() -> MockPermission {
    MockPermission {
        allow_client_mock_response: true,
    }
}

#[test]
fn decodes_allowed_client_mock_fields() {
    let body = br#"{"model":"m","messages":[],"mock_response":"client","mock_tool_calls":[]}"#;
    let request = ChatRequest::parse(body).unwrap();
    let (response, calls) = client_mock(&request.passthrough, allowed()).expect("mock fields");
    assert_eq!(response, Some(MockResponse::Text(String::from("client"))));
    assert_eq!(calls, Some(Vec::new()));
}

#[test]
fn decodes_a_client_tool_call_with_litellm_defaults() {
    let body = br#"{"model":"m","messages":[],"mock_tool_calls":[{"index":0,"function":{"name":"f","parameters":{"a":1}}}]}"#;
    let request = ChatRequest::parse(body).unwrap();
    let (_, calls) = client_mock(&request.passthrough, allowed()).expect("tool calls");
    let call = &calls.expect("tool calls")[0];
    assert_eq!(call.id, None);
    assert_eq!(call.kind, None);
    assert_eq!(call.function.arguments.as_deref(), Some(r#"{"a":1}"#));
}

fn capture_warnings<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, run);
    (result, read_to_string(log.path()).unwrap())
}

#[test]
fn encodes_object_arguments_as_json_text() {
    let body = br#"{"model":"m","messages":[],"mock_tool_calls":[{"function":{"name":"f","arguments":{"a":1}}}]}"#;
    let request = ChatRequest::parse(body).unwrap();
    let (_, calls) = client_mock(&request.passthrough, allowed()).expect("tool calls");
    let call = &calls.expect("tool calls")[0];
    assert_eq!(call.function.arguments.as_deref(), Some(r#"{"a":1}"#));
}
#[test]
fn a_null_client_mock_field_reads_as_absent() {
    let body = br#"{"model":"m","messages":[],"mock_response":null,"mock_tool_calls":null}"#;
    let request = ChatRequest::parse(body).unwrap();
    let (response, calls) = client_mock(&request.passthrough, allowed()).expect("null fields");
    assert_eq!(response, None);
    assert_eq!(calls, None);
}

#[rstest]
#[case::null_arguments(r#"{"function":{"name":"f","arguments":null}}"#, "")]
#[case::null_arguments_with_parameters(
    r#"{"function":{"name":"f","arguments":null,"parameters":{"a":1}}}"#,
    r#"{"a":1}"#
)]
#[case::arguments_win_over_parameters(
    r#"{"function":{"name":"f","arguments":"{}","parameters":{"a":1}}}"#,
    "{}"
)]
#[case::extra_function_keys_are_ignored(
    r#"{"function":{"name":"f","arguments":"{}","description":"d"}}"#,
    "{}"
)]
fn function_arguments_follow_litellm(#[case] call: &str, #[case] arguments: &str) {
    let body = format!(r#"{{"model":"m","messages":[],"mock_tool_calls":[{call}]}}"#);
    let request = ChatRequest::parse(body.as_bytes()).unwrap();
    let (_, calls) = client_mock(&request.passthrough, allowed()).expect("tool calls");
    let decoded = &calls.expect("tool calls")[0];
    assert_eq!(decoded.function.arguments.as_deref(), Some(arguments));
}

#[test]
fn a_client_tool_call_index_is_ignored() {
    let body = br#"{"model":"m","messages":[],"mock_tool_calls":[{"index":2,"function":{"name":"f","arguments":"{}"}}]}"#;
    let request = ChatRequest::parse(body).unwrap();
    let (_, calls) = client_mock(&request.passthrough, allowed()).expect("tool calls");
    assert_eq!(
        calls.expect("tool calls")[0].function.arguments.as_deref(),
        Some("{}")
    );
}

#[test]
fn a_client_tool_call_without_a_name_is_a_body_error() {
    let body =
        br#"{"model":"m","messages":[],"mock_tool_calls":[{"function":{"arguments":"{}"}}]}"#;
    let request = ChatRequest::parse(body).unwrap();
    client_mock(&request.passthrough, allowed()).expect_err("a nameless tool call");
}

#[test]
fn a_denied_client_mock_field_is_stripped_with_a_warning() {
    let choices: Value = serde_json::from_str(r#"{"choices": [{"logprobs": null}]}"#).unwrap();
    let calls: Value = serde_json::from_str(r#"[{"function": {"arguments": "{}"}}]"#).unwrap();
    let fields = IndexMap::from([
        (String::from("mock_response"), choices),
        (String::from("mock_tool_calls"), calls),
    ]);
    let (stripped, warnings) = capture_warnings(|| client_mock(&fields, MockPermission::default()));
    assert_eq!(stripped.expect("strip"), (None, None));
    assert!(warnings.contains("mock_response"), "{warnings}");
    assert!(warnings.contains("mock_tool_calls"), "{warnings}");
}

#[rstest]
#[case::not_json("{")]
#[case::unknown_role(r#"{"messages":[{"role":"robot","content":"hi"}]}"#)]
#[case::no_messages(r#"{"model":"m"}"#)]
fn refuses_bodies_that_are_not_chat_requests(#[case] body: &str) {
    let error = ChatRequest::parse(body.as_bytes()).unwrap_err();
    assert!(matches!(error, RequestError::Body { .. }));
}

#[rstest]
fn replays_the_chat_request_corpus(#[files("fuzz/corpus/chat_request/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _parsed = ChatRequest::parse(&bytes);
}

#[rstest]
#[case::prefixed("openai/gpt-4o", "gpt-4o")]
#[case::bare("gpt-4o", "gpt-4o")]
#[case::nested("azure/eu/gpt-4o", "eu/gpt-4o")]
fn mock_model_strips_the_provider_prefix(#[case] given: &str, #[case] expected: &str) {
    assert_eq!(mock_model(given), expected);
}

#[rstest]
#[case::absent(None, "stop", None)]
#[case::kept(Some("stop"), "stop", None)]
#[case::mapped(Some("max_tokens"), "length", Some("max_tokens"))]
#[case::tool_use(Some("tool_use"), "tool_calls", Some("tool_use"))]
fn choice_finish_reason_keeps_the_raw_value_when_mapped(
    #[case] given: Option<&str>,
    #[case] expected: &str,
    #[case] native: Option<&str>,
) {
    let (reason, extra) = choice_finish_reason(given);
    assert_eq!(reason, expected);
    let kept = extra.get("native_finish_reason").and_then(Value::as_str);
    assert_eq!(kept, native);
    assert_eq!(extra.len(), usize::from(native.is_some()));
}
