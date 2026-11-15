#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::chat::{ChatRequest, RequestError};

#[test]
fn a_null_disable_fallbacks_leaves_fallbacks_on() {
    let body = br#"{"model":"m","messages":[],"disable_fallbacks":null}"#;
    let request = ChatRequest::parse(body).unwrap();
    assert_ne!(request.disable_fallbacks, Some(true));
    let disabled = br#"{"model":"m","messages":[],"disable_fallbacks":true}"#;
    let disabled_request = ChatRequest::parse(disabled).unwrap();
    assert_eq!(disabled_request.disable_fallbacks, Some(true));
}

#[test]
fn the_category_fallback_overrides_are_read_and_never_forwarded() {
    let body = br#"{"model":"m","messages":[],"context_window_fallbacks":[{"gpt-4":["long-ctx"]}],"content_policy_fallbacks":[{"gpt-4":[{"model":"safe","messages":[]}]}]}"#;
    let request = ChatRequest::parse(body).unwrap();
    assert_eq!(
        serde_json::to_string(&request.context_window_fallbacks).unwrap(),
        r#"[{"gpt-4":["long-ctx"]}]"#
    );
    assert_eq!(
        serde_json::to_string(&request.content_policy_fallbacks).unwrap(),
        r#"[{"gpt-4":[{"model":"safe","messages":[]}]}]"#
    );
    let written = serde_json::to_string(&request).unwrap();
    assert!(!written.contains("context_window_fallbacks"));
    assert!(!written.contains("content_policy_fallbacks"));
    assert!(!written.contains("disable_fallbacks"));
}

#[test]
fn the_request_fallbacks_override_is_read_typed_and_never_forwarded() {
    let body = br#"{"model":"m","messages":[],"fallbacks":["a",{"b":["c"]}]}"#;
    let request = ChatRequest::parse(body).unwrap();
    assert_eq!(
        serde_json::to_string(&request.fallbacks).unwrap(),
        r#"["a",{"b":["c"]}]"#
    );
    assert!(
        !serde_json::to_string(&request)
            .unwrap()
            .contains("fallbacks")
    );
    let empty = ChatRequest::parse(br#"{"model":"m","messages":[],"fallbacks":[]}"#).unwrap();
    assert_eq!(empty.fallbacks, Some(Vec::new()));
    let absent = ChatRequest::parse(br#"{"model":"m","messages":[]}"#).unwrap();
    assert_eq!(absent.fallbacks, None);
    let wrong = ChatRequest::parse(br#"{"model":"m","messages":[],"fallbacks":"x"}"#).unwrap_err();
    assert!(matches!(wrong, RequestError::Body { .. }));
}

#[test]
fn a_request_api_key_is_accepted_and_returns_as_a_secret_string() {
    let (_request, parsed) =
        ChatRequest::parse_with_api_key(br#"{"model":"m","messages":[],"api_key":"sk-caller"}"#)
            .unwrap();
    let key = parsed.expect("the request key");
    assert_eq!(key.expose_secret(), "sk-caller");
    assert!(!format!("{key:?}").contains("sk-caller"));
    let (_absent, absent_key) =
        ChatRequest::parse_with_api_key(br#"{"model":"m","messages":[]}"#).unwrap();
    assert!(absent_key.is_none());
    let (_null, null_key) =
        ChatRequest::parse_with_api_key(br#"{"model":"m","messages":[],"api_key":null}"#).unwrap();
    assert!(null_key.is_none());
}

#[test]
fn the_request_api_key_never_stays_on_the_request() {
    let (request, _key) = ChatRequest::parse_with_api_key(
        br#"{"model":"m","messages":[],"api_key":"sk-caller","metadata":{"a":1}}"#,
    )
    .unwrap();
    let body = serde_json::to_string(&request).unwrap();
    assert!(!body.contains("sk-caller") && !body.contains("api_key"));
    assert!(body.contains("metadata"));
    assert!(!format!("{request:?}").contains("sk-caller"));
}

#[test]
fn a_plain_parse_still_refuses_a_request_api_key() {
    let error =
        ChatRequest::parse(br#"{"model":"m","messages":[],"api_key":"sk-caller"}"#).unwrap_err();
    assert!(matches!(&error, RequestError::Unsupported { name } if name == "api_key"));
    assert!(!error.to_string().contains("sk-caller"));
}

#[test]
fn a_non_string_api_key_is_refused_by_name() {
    let error =
        ChatRequest::parse_with_api_key(br#"{"model":"m","messages":[],"api_key":5}"#).unwrap_err();
    assert!(matches!(error, RequestError::FieldType { name } if name == "api_key"));
    assert!(error.to_string().contains("api_key"));
}
#[rstest]
#[case("api_base")]
#[case("base_url")]
fn a_request_destination_stays_refused(#[case] name: &str) {
    let body = format!(r#"{{"model":"m","messages":[],"{name}":"https://caller.example/v1"}}"#);
    let error = ChatRequest::parse(body.as_bytes()).unwrap_err();
    assert!(matches!(error, RequestError::Unsupported { name: refused } if refused == name));
}
