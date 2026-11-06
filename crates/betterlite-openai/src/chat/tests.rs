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

use super::{ChatRequest, MessageContent, RequestError, Role, Stop, ToolChoice, client_mock};

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
