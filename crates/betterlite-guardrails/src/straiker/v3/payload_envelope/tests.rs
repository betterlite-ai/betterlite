#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde::Deserialize;

use super::{PayloadInput, Phase, payload};
use crate::straiker::v3::answer_shape::Response;
use crate::straiker::v3::fixtures::{Routes, Translator, context, identity, parsed};

#[derive(Deserialize)]
struct Setup {
    #[serde(default)]
    request_data: Object,
    request_body: Object,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    texts: Vec<String>,
    #[serde(default)]
    chat: Option<Object>,
}

const BODY: &str = r#"{"model":"gpt-4o","messages":[{"role":"system","content":"rules"},{"role":"user","content":"hi"}]}"#;

fn built(phase: Phase, source: &str) -> Json {
    let setup: Setup = serde_json::from_str(source).unwrap();
    let mut context = context(setup.session.as_deref());
    context.model = setup.model;
    let principal = identity(setup.user.as_deref());
    let response = setup
        .chat
        .map_or(Response::Other(Json::Null), Response::Chat);
    let input = PayloadInput {
        context: &context,
        identity: &principal,
        texts: &setup.texts,
        response: &response,
        request_data: &setup.request_data,
        routes: &Routes,
        translator: &Translator,
    };
    let mut fields = payload(&input, phase, &setup.request_body).unwrap();
    // Straiker receives the answer as the raw JSON text a response body carries, never as a nested object, so the wire type is checked before the text is compared as the JSON it holds.
    if let Some(value) = fields.get("sse") {
        let Json::Text(answer) = value else {
            panic!("the sse field must be the answer's JSON text");
        };
        let decoded = parsed(answer);
        fields.insert("sse".to_owned(), decoded);
    }
    Json::Object(fields)
}

fn with_body(setup: &str) -> String {
    setup.replace("BODY", BODY)
}

#[rstest]
#[case::request_phase_is_the_provider_body_plus_session_and_user(
    r#"{"request_body":BODY,"user":"alice"}"#,
    r#"{"model":"gpt-4o","messages":[{"role":"system","content":"rules"},{"role":"user","content":"hi"}],"session_id":"SESSION","original":{"processed":{"Meta":{"user":"alice"}}}}"#
)]
#[case::litellm_session_is_relayed(
    r#"{"request_body":BODY,"session":"meta-1"}"#,
    r#"{"model":"gpt-4o","messages":[{"role":"system","content":"rules"},{"role":"user","content":"hi"}],"session_id":"meta-1"}"#
)]
#[case::client_session_header_wins(
    r#"{"request_body":BODY,"session":"meta-1","request_data":{"proxy_server_request":{"headers":{"x-claude-code-session-id":"cc-9"}}}}"#,
    r#"{"model":"gpt-4o","messages":[{"role":"system","content":"rules"},{"role":"user","content":"hi"}],"session_id":"cc-9"}"#
)]
#[case::session_replaces_the_body_session_in_place(
    r#"{"request_body":{"session_id":"mine","model":"m"},"session":"meta-1"}"#,
    r#"{"session_id":"meta-1","model":"m"}"#
)]
#[case::nothing_to_group_on(r#"{"request_body":{"model":"m"}}"#, r#"{"model":"m"}"#)]
fn builds_request_phase(#[case] setup: &str, #[case] expected: &str) {
    let built_body = built(Phase::Request, &with_body(setup));
    let session = derived_session(&built_body);
    if expected.contains("SESSION") {
        assert!(session.starts_with("litellm-"));
    }
    let wanted = parsed(&with_body(expected).replace("SESSION", &session));
    assert_eq!(built_body, wanted);
}

fn derived_session(body: &Json) -> String {
    body.as_object()
        .and_then(|fields| fields.get("session_id"))
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[rstest]
#[case::answer_beside_the_request(
    r#"{"request_body":{"model":"m"},"model":"gpt-4o","session":"s","chat":{"id":"r1","choices":[]}}"#,
    r#"{"straiker_phase":"response-sync","model":"gpt-4o","request":{"model":"m"},"sse":{"id":"r1","choices":[]},"session_id":"s"}"#
)]
#[case::streamed_texts_become_a_chat_completion(
    r#"{"request_body":{"model":"m"},"model":"m","session":"s","texts":["a","","b"]}"#,
    r#"{"straiker_phase":"response-sync","model":"m","request":{"model":"m"},"sse":{"object":"chat.completion","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"a\nb"}}]},"session_id":"s"}"#
)]
#[case::real_answer_beats_streamed_texts(
    r#"{"request_body":{"model":"m"},"model":"m","session":"s","texts":["a"],"chat":{"id":"r1"}}"#,
    r#"{"straiker_phase":"response-sync","model":"m","request":{"model":"m"},"sse":{"id":"r1"},"session_id":"s"}"#
)]
#[case::empty_answer_falls_to_texts(
    r#"{"request_body":{"model":"m"},"model":"m","session":"s","texts":["a"],"chat":{}}"#,
    r#"{"straiker_phase":"response-sync","model":"m","request":{"model":"m"},"sse":{"object":"chat.completion","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"a"}}]},"session_id":"s"}"#
)]
#[case::no_answer_and_no_texts(
    r#"{"request_body":{"model":"m"},"model":"m","session":"s","texts":["",""]}"#,
    r#"{"straiker_phase":"response-sync","model":"m","request":{"model":"m"},"session_id":"s"}"#
)]
#[case::messages_answer_is_translated(
    r#"{"request_body":{"model":"m"},"model":"claude","session":"s","user":"alice","chat":{"id":"r1"},"request_data":{"metadata":{"user_api_key_request_route":"/v1/messages"}}}"#,
    r#"{"straiker_phase":"response-sync","model":"claude","request":{"model":"m"},"sse":{"id":"r1","type":"message","model":"claude"},"session_id":"s","original":{"processed":{"Meta":{"user":"alice"}}}}"#
)]
#[case::unknown_model_is_null(
    r#"{"request_body":{"model":"m"},"session":"s"}"#,
    r#"{"straiker_phase":"response-sync","model":null,"request":{"model":"m"},"session_id":"s"}"#
)]
fn builds_response_phase(#[case] setup: &str, #[case] expected: &str) {
    assert_eq!(built(Phase::Response, setup), parsed(expected));
}

#[test]
fn response_phase_keeps_wire_order() {
    let setup =
        r#"{"request_body":{"model":"m"},"model":"m","session":"s","texts":["a"],"user":"u"}"#;
    let body = built(Phase::Response, setup);
    let keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "straiker_phase",
            "model",
            "request",
            "sse",
            "session_id",
            "original"
        ]
    );
}
