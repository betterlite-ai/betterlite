#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::{ChatRequest, MessageContent, RequestError, Role, Stop, ToolChoice};

#[test]
fn parses_typed_fields_and_keeps_unknown_parameters_for_forwarding() {
    let body = br#"{"messages":[{"role":"user","content":[{"type":"text","text":"hi"}],"cache_control":{"type":"ephemeral"}}],"stop":"\n","tool_choice":"auto","n":2,"temperature":1,"vendor_flag":true}"#;
    let request = ChatRequest::parse(body).unwrap();
    assert_eq!(request.model, None);
    assert_eq!(request.choice_count, Some(2));
    assert_eq!(request.stop, Some(Stop::One("\n".to_owned())));
    assert_eq!(request.tool_choice, Some(ToolChoice::Mode("auto".to_owned())));
    let message = request.messages.first().unwrap();
    assert_eq!(message.role, Role::User);
    assert!(matches!(message.content, Some(MessageContent::Parts(_))));
    assert_eq!(message.passthrough.keys().collect::<Vec<_>>(), ["cache_control"]);
    assert_eq!(request.passthrough.keys().collect::<Vec<_>>(), ["vendor_flag"]);
    let written = serde_json::to_value(&request).unwrap();
    assert_eq!(written.get("temperature").unwrap(), 1);
    assert_eq!(written.get("n").unwrap(), 2);
}

#[rstest]
#[case::api_base("api_base")]
#[case::fallbacks("fallbacks")]
#[case::mock_response("mock_response")]
#[case::extra_headers("extra_headers")]
fn refuses_litellm_request_keys_it_does_not_act_on(#[case] name: &str) {
    let body = format!(r#"{{"model":"m","messages":[],"{name}":"x"}}"#);
    let error = ChatRequest::parse(body.as_bytes()).unwrap_err();
    assert!(matches!(error, RequestError::Unsupported { name: refused } if refused == name));
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
