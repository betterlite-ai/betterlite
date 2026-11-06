use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::spend_sanitize::REDACTED_BY_LITELM;
use crate::spend_sanitize::request_body::sanitize_request_body;
use crate::spend_sanitize::value::SpendValue;

use super::support::{lookup, mapping, number, object, sequence, text, value};

fn redacted() -> Json {
    Json::Text(REDACTED_BY_LITELM.to_owned())
}

fn sanitize(body: SpendValue) -> Json {
    let SpendValue::Mapping(mapping) = body else {
        panic!("a request body is a mapping")
    };
    sanitize_request_body(&mapping, 2048)
}

#[test]
fn masks_credentials_and_drops_secret_fields_at_every_depth() {
    let body = value(
        r#"{"api_key":"sk-live-1","apikey":"sk-apikey-2","secret_fields":{"authorization":"Bearer x"},"nested":{"password":"hunter2","secret_fields":{"k":"v"}},"max_tokens":17}"#,
    );
    let sanitized = sanitize(body);
    assert_eq!(lookup(&sanitized, "api_key"), &redacted());
    assert_eq!(lookup(&sanitized, "apikey"), &redacted());
    assert!(!object(&sanitized).contains_key("secret_fields"));
    assert_eq!(lookup(&sanitized, "max_tokens"), &number(17));
    assert_eq!(lookup(&sanitized, "nested.password"), &redacted());
    assert!(!object(lookup(&sanitized, "nested")).contains_key("secret_fields"));
}

#[test]
fn keeps_user_content_when_a_credential_key_sits_inside_tool_payloads() {
    let body = value(
        r#"{"messages":[{"type":"tool_use","input":{"api_key":"sk-keep","nested":{"password":"keep"}}},{"type":"tool_result","role":"tool","content":[{"api_key":"sk-keep-2"}],"output":{"token":"keep"}},{"role":"tool","content":{"key":"keep"}}],"arguments":{"api_key":"sk-keep-3"},"logprobs":{"token":"keep"},"tools":[{"function":{"parameters":{"api_key":"sk-mask"}}}]}"#,
    );
    let sanitized = sanitize(body);
    assert_eq!(
        lookup(&sanitized, "messages.0.input.api_key"),
        &Json::Text("sk-keep".to_owned())
    );
    assert_eq!(
        lookup(&sanitized, "messages.0.input.nested.password"),
        &Json::Text("keep".to_owned())
    );
    assert_eq!(
        lookup(&sanitized, "messages.1.content.0.api_key"),
        &Json::Text("sk-keep-2".to_owned())
    );
    assert_eq!(
        lookup(&sanitized, "messages.1.output.token"),
        &Json::Text("keep".to_owned())
    );
    assert_eq!(
        lookup(&sanitized, "messages.2.content.key"),
        &Json::Text("keep".to_owned())
    );
    assert_eq!(
        lookup(&sanitized, "arguments.api_key"),
        &Json::Text("sk-keep-3".to_owned())
    );
    assert_eq!(
        lookup(&sanitized, "logprobs.token"),
        &Json::Text("keep".to_owned())
    );
    assert_eq!(
        lookup(&sanitized, "tools.0.function.parameters.api_key"),
        &redacted()
    );
}

#[test]
fn keeps_stripping_secret_fields_inside_tool_content() {
    let body = value(
        r#"{"messages":[{"type":"tool_use","input":{"secret_fields":{"k":"v"},"api_key":"keep"}}]}"#,
    );
    let sanitized = sanitize(body);
    assert_eq!(
        lookup(&sanitized, "messages.0.input.api_key"),
        &Json::Text("keep".to_owned())
    );
    assert!(!object(lookup(&sanitized, "messages.0.input")).contains_key("secret_fields"));
}

#[test]
fn leaves_non_string_credential_values_unmasked() {
    let body = value(r#"{"token_count":5,"api_key":["sk-item"],"key":{"api_key":"sk-nested"}}"#);
    let sanitized = sanitize(body);
    assert_eq!(lookup(&sanitized, "token_count"), &number(5));
    assert_eq!(
        lookup(&sanitized, "api_key.0"),
        &Json::Text("sk-item".to_owned())
    );
    assert_eq!(lookup(&sanitized, "key.api_key"), &redacted());
}

#[test]
fn replaces_a_repeated_mapping_with_an_empty_object_across_siblings() {
    let shared = mapping(7, vec![("api_key", text("sk-once"))]);
    let body = mapping(
        1,
        vec![
            ("first", shared.clone()),
            ("second", shared.clone()),
            ("third", mapping(2, vec![("inner", shared)])),
        ],
    );
    let sanitized = sanitize(body);
    assert_eq!(lookup(&sanitized, "first.api_key"), &redacted());
    assert_eq!(lookup(&sanitized, "second"), &Json::Object(IndexMap::new()));
    assert_eq!(
        lookup(&sanitized, "third.inner"),
        &Json::Object(IndexMap::new())
    );
}

#[test]
fn truncates_strings_at_every_depth() {
    let body = mapping(1, vec![("messages", sequence(2, vec![text("abcdefghij")]))]);
    let sanitized = sanitize(body);
    let Json::Text(truncated) = lookup(&sanitized, "messages.0") else {
        panic!("a list string stays text")
    };
    assert!(truncated.starts_with('a'));
    assert!(truncated.ends_with("ij"));
    assert!(truncated.contains("litellm_truncated"));
}
