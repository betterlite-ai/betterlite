use betterlite_openai::openai_base_config::Headers;

use super::support;
use crate::openai_dispatch::call::{ChatCall, Composition};
use crate::openai_dispatch::fixtures::answer::{FixtureAnswer, RecordedRequest};
use crate::openai_dispatch::fixtures::logger::FixtureLogger;

const BODY: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"extra_headers":{"x-body":"1"}}"#;

fn headers(fields: &[(&str, &str)]) -> Headers {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn call(extra: Option<Headers>) -> ChatCall {
    ChatCall {
        model: String::from("gpt-4o"),
        request: support::raw_chat_request(BODY),
        credentials: support::credentials(
            Some("https://explicit.test/v1"),
            Some("sk-explicit"),
            None,
        ),
        headers: Some(headers(&[("x-header", "h")])),
        extra_headers: extra,
        timeout: None,
        metadata: None,
    }
}

fn extra() -> Headers {
    headers(&[("x-header", "e"), ("x-other", "1")])
}

fn configured() -> Composition {
    let mut composition = support::composition();
    composition.globals.organization = Some(String::from("org-global"));
    composition
}

async fn sent(composition: &Composition, extra: Option<Headers>) -> Vec<RecordedRequest> {
    let answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let _result = support::dispatch_chat(composition, call(extra), &answer, &mut logger)
        .await
        .expect("the chat dispatch");
    answer.requests()
}

// The default sends the organization and the extra headers; the experimental handler receives neither.
pub(super) async fn check_branches() {
    let mut composition = configured();
    assert!(!composition.provider.experimental_handler);
    let sdk = sent(&composition, Some(extra())).await;
    assert_eq!(sdk[0].header("openai-organization"), Some("org-global"));
    assert_eq!(sdk[0].header("x-header"), Some("e"));
    assert_eq!(sdk[0].header("x-other"), Some("1"));
    assert_eq!(sdk[0].header("content-type"), Some("application/json"));
    // the extra headers travel as HTTP headers, never inside the body
    assert!(!support::body_fields(&sdk[0].body).contains_key("extra_headers"));
    assert_eq!(sdk[0].header("x-body"), None);
    composition.provider.experimental_handler = true;
    let experimental = sent(&composition, Some(extra())).await;
    assert_eq!(experimental[0].header("openai-organization"), None);
    assert_eq!(
        experimental[0].header("authorization"),
        Some("Bearer sk-explicit")
    );
    assert_eq!(experimental[0].header("x-header"), Some("e"));
    assert_eq!(experimental[0].header("x-other"), Some("1"));
    assert!(!support::body_fields(&experimental[0].body).contains_key("extra_headers"));
}

// An empty extra map is present, so the normal branch inserts it over any extra headers the body carried.
pub(super) async fn check_empty_extra_map_overwrites() {
    let sent = sent(&configured(), Some(Headers::new())).await;
    // the caller map still reaches the wire, and the body's extra headers do not
    assert_eq!(sent[0].header("x-header"), Some("h"));
    assert_eq!(sent[0].header("x-body"), None);
}

pub(super) async fn check_all() {
    check_branches().await;
    check_empty_extra_map_overwrites().await;
}
