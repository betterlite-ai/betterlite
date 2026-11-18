use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;

use super::support;
use crate::openai_dispatch::call::{ChatCall, Composition, Credentials};
use crate::openai_dispatch::fixtures::answer::{FixtureAnswer, RecordedRequest};
use crate::openai_dispatch::fixtures::logger::FixtureLogger;
use crate::openai_dispatch::headers::resolved_headers;

fn headers(fields: &[(&str, &str)]) -> Headers {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn chat_call(body: &str, credentials: Credentials) -> ChatCall {
    ChatCall {
        model: String::from("gpt-4o"),
        request: support::chat_request(body),
        credentials,
        headers: None,
        extra_headers: None,
        timeout: None,
        metadata: None,
    }
}

fn globals_composition(api_key: Option<&str>, openai_key: Option<&str>) -> Composition {
    let mut composition = support::composition();
    let identity =
        support::credentials(Some("https://global.test/v1"), api_key, Some("org-global"));
    composition.globals = support::globals(identity, openai_key);
    composition.env_api_key = Some(SecretString::from("sk-env"));
    composition
}

async fn dispatched(composition: &Composition, credentials: Credentials) -> Vec<RecordedRequest> {
    let call = chat_call(
        r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
        credentials,
    );
    let answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let _result = support::dispatch_chat(composition, call, &answer, &mut logger)
        .await
        .expect("the chat dispatch");
    answer.requests()
}

// The explicit values the route resolved win over every global and the configured defaults.
pub(super) async fn check_explicit_wins() {
    let mut composition = support::composition();
    let identity = support::credentials(
        Some("https://global.test/v1"),
        Some("sk-global"),
        Some("org-global"),
    );
    composition.globals = support::globals(identity, Some("sk-openai-global"));
    composition.env_api_key = Some(SecretString::from("sk-env"));
    composition.credentials.api_base = String::from("https://settings.test/v1");
    composition.credentials.organization = Some(String::from("org-settings"));
    let call = chat_call(
        r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
        support::credentials(
            Some("https://explicit.test/v1"),
            Some("sk-explicit"),
            Some("org-explicit"),
        ),
    );
    let answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let _result = support::dispatch_chat(&composition, call, &answer, &mut logger)
        .await
        .expect("the chat dispatch");
    let sent = answer.requests();
    assert_eq!(sent[0].url, "https://explicit.test/v1/chat/completions");
    assert_eq!(sent[0].header("authorization"), Some("Bearer sk-explicit"));
    assert_eq!(sent[0].header("openai-organization"), Some("org-explicit"));
}

// With no explicit value the globals name the call, and an empty explicit string is not a value.
pub(super) async fn check_globals_name_the_call() {
    let composition = globals_composition(Some("sk-global"), Some("sk-openai-global"));
    let credentials = support::credentials(Some(""), Some(""), Some(""));
    let sent = dispatched(&composition, credentials).await;
    assert_eq!(sent[0].url, "https://global.test/v1/chat/completions");
    assert_eq!(sent[0].header("authorization"), Some("Bearer sk-global"));
    assert_eq!(sent[0].header("openai-organization"), Some("org-global"));
}

// openai_key backs the call when litellm.api_key is unset.
pub(super) async fn check_openai_key_backs_the_call() {
    let composition = globals_composition(None, Some("sk-openai-global"));
    let sent = dispatched(&composition, support::credentials(None, None, None)).await;
    assert_eq!(
        sent[0].header("authorization"),
        Some("Bearer sk-openai-global")
    );
}

// The environment key is the last resort, and the configured organization names the call.
pub(super) async fn check_env_key_and_settings_organization() {
    let mut composition = globals_composition(None, None);
    composition.globals.organization = None;
    composition.credentials.organization = Some(String::from("org-settings"));
    let sent = dispatched(&composition, support::credentials(None, None, None)).await;
    assert_eq!(sent[0].header("authorization"), Some("Bearer sk-env"));
    assert_eq!(sent[0].header("openai-organization"), Some("org-settings"));
}

// completion() takes a truthy supplied map, then the extra map, then litellm.headers, and the extra map wins a shared name.
pub(super) fn check_header_resolution() {
    let mut composition = support::composition();
    let supplied = headers(&[("x-supplied", "s"), ("x-shared", "supplied")]);
    let extra = headers(&[("x-extra", "e"), ("x-shared", "extra")]);
    assert_eq!(
        resolved_headers(&composition, Some(&supplied), Some(&extra)),
        Some(headers(&[
            ("x-extra", "e"),
            ("x-shared", "extra"),
            ("x-supplied", "s"),
        ]))
    );
    // an empty supplied map is falsy and lets the extra map stand alone
    assert_eq!(
        resolved_headers(&composition, Some(&Headers::new()), Some(&extra)),
        Some(extra.clone())
    );
    // an absent extra map keeps the supplied one as it is
    assert_eq!(
        resolved_headers(&composition, Some(&supplied), None),
        Some(supplied.clone())
    );
    composition.globals_headers = Some(headers(&[("x-global", "g")]));
    assert_eq!(
        resolved_headers(&composition, None, None),
        Some(headers(&[("x-global", "g")]))
    );
    // an empty extra map is falsy as well, so the global map still stands
    assert_eq!(
        resolved_headers(&composition, None, Some(&Headers::new())),
        Some(headers(&[("x-global", "g")]))
    );
    assert_eq!(
        resolved_headers(&composition, None, Some(&extra)),
        Some(extra)
    );
    assert_eq!(resolved_headers(&support::composition(), None, None), None);
}

// Defaults fill only absent keys: an explicit null, false, 0 or empty list stays as the caller wrote it.
pub(super) async fn check_defaults_fill_absent_keys_only() {
    let mut composition = support::composition();
    composition.chat_defaults = support::chat_defaults(
        r#"{"temperature":0.5,"max_tokens":7,"n":2,"stop":["\n"],"presence_penalty":0.1,"top_p":0.9}"#,
    );
    let call = chat_call(
        r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"max_tokens":0,"n":0,"stop":[],"presence_penalty":0.0,"logprobs":false}"#,
        support::credentials(Some("https://explicit.test/v1"), Some("sk-explicit"), None),
    );
    let answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let _result = support::dispatch_chat(&composition, call, &answer, &mut logger)
        .await
        .expect("the chat dispatch");
    let body = support::body_fields(&answer.requests()[0].body);
    assert_eq!(
        support::field(&body, "max_tokens"),
        &support::number_value(0)
    );
    assert_eq!(support::field(&body, "n"), &support::number_value(0));
    assert_eq!(
        support::field(&body, "presence_penalty"),
        &JsonValue::Number(serde_json::Number::from_f64(0.0).expect("a number"))
    );
    assert_eq!(support::field(&body, "logprobs"), &JsonValue::Bool(false));
    let JsonValue::Array(stop) = support::field(&body, "stop") else {
        panic!("stop stays an array");
    };
    assert!(stop.is_empty());
    // an absent key took the configured default
    assert_eq!(
        support::field(&body, "top_p"),
        &JsonValue::Number(serde_json::Number::from_f64(0.9).expect("a number"))
    );
    assert_eq!(
        support::field(&body, "temperature"),
        &JsonValue::Number(serde_json::Number::from_f64(0.5).expect("a number"))
    );
}

pub(super) async fn check_all() {
    check_explicit_wins().await;
    check_globals_name_the_call().await;
    check_openai_key_backs_the_call().await;
    check_env_key_and_settings_organization().await;
    check_header_resolution();
    check_defaults_fill_absent_keys_only().await;
}
