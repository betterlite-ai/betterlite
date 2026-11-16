use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use rstest::rstest;

use crate::a2a_chat::error::A2aFailure;
use crate::a2a_chat::records::{A2aConfiguration, A2aMethod};
use crate::a2a_chat::request::{
    SUPPORTED_OPENAI_PARAMS, convert_messages_to_prompt, get_complete_url, map_openai_params,
    stream_requested, transform_request, validate_environment,
};

use super::support::{CountedIds, header_map, messages, object, secret};

#[test]
fn only_the_documented_openai_params_are_supported() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["stream", "temperature", "max_tokens", "top_p"]
    );
}

#[rstest]
#[case::true_stream(r#"{"stream":true}"#, r#"{"stream":true}"#)]
#[case::false_stream(r#"{"stream":false}"#, "{}")]
#[case::text_stream(r#"{"stream":"true"}"#, "{}")]
#[case::other_params(r#"{"temperature":0.2}"#, "{}")]
#[case::empty("{}", "{}")]
fn map_openai_params_copies_only_true_stream(#[case] non_default: &str, #[case] expected: &str) {
    let mut optional = Object::new();
    map_openai_params(&object(non_default), &mut optional);
    assert_eq!(optional, object(expected));
}

#[test]
fn validate_environment_adds_json_and_the_bearer_key() {
    let key = secret("sk-1");
    let validated = validate_environment(&BTreeMap::new(), Some(&key));
    assert_eq!(
        validated,
        header_map(&[
            ("Content-Type", "application/json"),
            ("Authorization", "Bearer sk-1")
        ])
    );
}

#[test]
fn validate_environment_without_a_key_adds_only_json() {
    let validated = validate_environment(&BTreeMap::new(), None);
    assert_eq!(
        validated,
        header_map(&[("Content-Type", "application/json")])
    );
}

#[test]
fn an_empty_key_still_adds_the_bearer_header() {
    let key = secret("");
    let validated = validate_environment(&BTreeMap::new(), Some(&key));
    assert_eq!(
        validated,
        header_map(&[
            ("Content-Type", "application/json"),
            ("Authorization", "Bearer ")
        ])
    );
}

#[rstest]
#[case::lower("content-type")]
#[case::upper("Content-Type")]
fn a_caller_content_type_is_kept(#[case] name: &str) {
    let given = header_map(&[(name, "text/plain")]);
    let validated = validate_environment(&given, None);
    assert_eq!(validated, header_map(&[(name, "text/plain")]));
}

#[test]
fn a_caller_authorization_is_replaced_by_the_key() {
    let given = header_map(&[("Authorization", "old")]);
    let key = secret("k");
    let validated = validate_environment(&given, Some(&key));
    assert_eq!(
        validated,
        header_map(&[
            ("Authorization", "Bearer k"),
            ("Content-Type", "application/json")
        ])
    );
}

#[rstest]
#[case::as_is("http://host", "http://host")]
#[case::one_slash("http://host/", "http://host")]
#[case::many_slashes("http://host///", "http://host")]
#[case::inner_slash("http://host/a/", "http://host/a")]
#[case::root("/", "")]
fn get_complete_url_strips_trailing_slashes(#[case] base: &str, #[case] expected: &str) {
    assert_eq!(get_complete_url(Some(base)).unwrap(), expected);
}

#[test]
fn get_complete_url_without_an_api_base_names_the_provider() {
    let error = get_complete_url(None).unwrap_err();
    assert!(matches!(error, A2aFailure::MissingApiBase));
    assert_eq!(error.to_string(), "api_base is required for A2A provider");
}

#[rstest]
#[case::two_messages(
    r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"there"}]"#,
    "user: hi\nassistant: there"
)]
#[case::empty_text_skipped(
    r#"[{"role":"user","content":"hi"},{"role":"assistant","content":""}]"#,
    "user: hi"
)]
#[case::only_empty(r#"[{"role":"user","content":""}]"#, "")]
#[case::list_content(
    r#"[{"role":"user","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}]"#,
    "user: ab"
)]
#[case::developer_role(r#"[{"role":"developer","content":"x"}]"#, "developer: x")]
fn convert_messages_to_prompt_joins_non_empty_messages(#[case] wire: &str, #[case] expected: &str) {
    assert_eq!(convert_messages_to_prompt(&messages(wire)), expected);
}

#[test]
fn transform_request_sends_a_blocking_message_send() {
    let ids = CountedIds::new();
    let body = transform_request(
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        &Object::new(),
        &ids,
    )
    .unwrap();
    assert_eq!(body.jsonrpc, "2.0");
    assert_eq!(body.id, "id-0");
    assert_eq!(body.method, A2aMethod::Send);
    assert_eq!(body.params.message.kind, "message");
    assert_eq!(body.params.message.role, "user");
    assert_eq!(body.params.message.message_id, "id-1");
    assert_eq!(body.params.message.parts.len(), 1);
    assert_eq!(body.params.message.parts[0].kind, "text");
    assert_eq!(body.params.message.parts[0].text, "user: hi");
    assert_eq!(
        body.params.configuration,
        Some(A2aConfiguration { blocking: true })
    );
}

#[test]
fn transform_request_streams_without_the_blocking_flag() {
    let ids = CountedIds::new();
    let options = object(r#"{"stream":true}"#);
    let body = transform_request(
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        &options,
        &ids,
    )
    .unwrap();
    assert_eq!(body.id, "id-0");
    assert_eq!(body.method, A2aMethod::Stream);
    assert_eq!(body.params.message.message_id, "id-1");
    assert_eq!(body.params.configuration, None);
}

#[test]
fn transform_request_draws_the_request_id_before_the_empty_messages_error() {
    let ids = CountedIds::new();
    let error = transform_request(&[], &Object::new(), &ids).unwrap_err();
    assert!(matches!(error, A2aFailure::MessagesRequired));
    assert_eq!(ids.next_id(), "id-1");
}

#[test]
fn transform_request_sends_an_empty_prompt_for_only_empty_messages() {
    let ids = CountedIds::new();
    let body = transform_request(
        &messages(r#"[{"role":"user","content":""}]"#),
        &Object::new(),
        &ids,
    )
    .unwrap();
    assert_eq!(body.params.message.parts[0].text, "");
}

#[rstest]
#[case::absent("{}", false)]
#[case::null(r#"{"stream":null}"#, false)]
#[case::true_value(r#"{"stream":true}"#, true)]
#[case::false_value(r#"{"stream":false}"#, false)]
fn stream_requested_reads_the_bool(#[case] wire: &str, #[case] expected: bool) {
    assert_eq!(stream_requested(&object(wire)).unwrap(), expected);
}

#[rstest]
#[case::text(r#"{"stream":"true"}"#)]
#[case::number(r#"{"stream":1}"#)]
#[case::list(r#"{"stream":[]}"#)]
#[case::object(r#"{"stream":{}}"#)]
fn a_non_bool_stream_is_rejected(#[case] wire: &str) {
    let error = stream_requested(&object(wire)).unwrap_err();
    assert!(matches!(error, A2aFailure::OptionType { field: "stream" }));
}
