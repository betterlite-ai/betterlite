use betterlite_openai::openai_base_config::Headers;

use super::support;
use crate::openai_dispatch::call::ChatCall;
use crate::openai_dispatch::fixtures::{FixtureAnswer, FixtureLogger};

fn headers(fields: &[(&str, &str)]) -> Headers {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn call(body: &str, extra: Option<Headers>) -> ChatCall {
    ChatCall {
        model: String::from("gpt-4o"),
        request: support::raw_chat_request(body),
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

pub async fn check() {
    let body = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"extra_headers":{"x-body":"1"}}"#;
    let extra = headers(&[("x-header", "e"), ("x-other", "1")]);

    // the default leaves the handler off: the SDK branch sends the organization and the extra headers
    let mut composition = support::composition();
    composition.globals.organization = Some(String::from("org-global"));
    assert!(!composition.provider.experimental_base_llm_http_handler);
    let answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let _result = support::dispatch_chat(
        &composition,
        call(body, Some(extra.clone())),
        &answer,
        &mut logger,
    )
    .await
    .expect("the chat dispatch");
    let sent = answer.requests();
    assert_eq!(sent[0].header("openai-organization"), Some("org-global"));
    assert_eq!(sent[0].header("x-header"), Some("e"));
    assert_eq!(sent[0].header("x-other"), Some("1"));
    assert_eq!(sent[0].header("content-type"), Some("application/json"));
    // the extra headers travel as HTTP headers, never inside the body
    let fields = support::body_fields(&sent[0].body);
    assert!(!fields.contains_key("extra_headers"));
    assert_eq!(sent[0].header("x-body"), None);

    // the experimental handler receives neither the organization nor an extra_headers insertion
    composition.provider.experimental_base_llm_http_handler = true;
    let answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let _result =
        support::dispatch_chat(&composition, call(body, Some(extra)), &answer, &mut logger)
            .await
            .expect("the chat dispatch");
    let sent = answer.requests();
    assert_eq!(sent[0].header("openai-organization"), None);
    assert_eq!(sent[0].header("authorization"), Some("Bearer sk-explicit"));
    assert_eq!(sent[0].header("x-header"), Some("e"));
    assert_eq!(sent[0].header("x-other"), Some("1"));
    let fields = support::body_fields(&sent[0].body);
    assert!(!fields.contains_key("extra_headers"));
}

// An empty extra map is present, so the normal branch inserts it over any extra headers the body carried.
pub async fn check_empty_extra_map_overwrites() {
    let mut composition = support::composition();
    composition.globals.organization = Some(String::from("org-global"));
    let body = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"extra_headers":{"x-body":"1"}}"#;
    let answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let _result = support::dispatch_chat(
        &composition,
        call(body, Some(Headers::new())),
        &answer,
        &mut logger,
    )
    .await
    .expect("the chat dispatch");
    let sent = answer.requests();
    // the caller map still reaches the wire, and the body's extra headers do not
    assert_eq!(sent[0].header("x-header"), Some("h"));
    assert_eq!(sent[0].header("x-body"), None);
}
