#![cfg(test)]

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::error::{FireworksError, MissingApiKey};
use super::{
    AUTHORIZATION_HEADER, CONTENT_TYPE_HEADER, FireworksEnv, SESSION_AFFINITY_HEADER,
    SessionMetadata, SessionParams, get_session_id, resolve_api_key, validate_environment,
    with_session_affinity,
};
use crate::openai_base_config::Headers;

fn key(text: &str) -> SecretString {
    SecretString::from(text)
}

fn env_with(
    fireworks: Option<&str>,
    ai: Option<&str>,
    joined: Option<&str>,
    token: Option<&str>,
) -> FireworksEnv {
    FireworksEnv {
        fireworks_api_key: fireworks.map(key),
        fireworks_ai_api_key: ai.map(key),
        fireworksai_api_key: joined.map(key),
        fireworks_ai_token: token.map(key),
    }
}

fn params(
    litellm: Option<&str>,
    session: Option<&str>,
    metadata: Option<SessionMetadata>,
) -> SessionParams {
    SessionParams {
        litellm_session_id: litellm.map(str::to_owned),
        session_id: session.map(str::to_owned),
        metadata,
    }
}

fn exposed(key: Option<&SecretString>) -> Option<&str> {
    key.map(ExposeSecret::expose_secret)
}

#[test]
fn resolve_api_key_takes_the_argument_before_every_env_name() {
    let argument = key("argument-key");
    let env = env_with(Some("first"), Some("second"), Some("third"), Some("fourth"));
    let resolved = resolve_api_key(Some(&argument), &env);
    assert_eq!(exposed(resolved.as_ref()), Some("argument-key"));
}

#[rstest]
#[case::fireworks_key(Some("one"), None, None, None, "one")]
#[case::fireworks_ai_key(None, Some("two"), None, None, "two")]
#[case::fireworksai_key(None, None, Some("three"), None, "three")]
#[case::token_last(None, None, None, Some("four"), "four")]
#[case::first_non_empty_wins(Some("one"), Some("two"), None, None, "one")]
#[case::empty_values_are_unset(Some(""), Some(""), Some("three"), None, "three")]
#[case::empty_argument_falls_through(Some(""), Some("one"), None, None, "one")]
fn resolve_api_key_reads_the_four_env_names_in_order(
    #[case] fireworks: Option<&str>,
    #[case] fireworks_ai: Option<&str>,
    #[case] fireworksai: Option<&str>,
    #[case] token: Option<&str>,
    #[case] expected: &str,
) {
    let argument = key("");
    let env = env_with(fireworks, fireworks_ai, fireworksai, token);
    assert_eq!(
        exposed(resolve_api_key(Some(&argument), &env).as_ref()),
        Some(expected)
    );
}

#[test]
fn resolve_api_key_returns_none_without_any_key() {
    let empty_argument = key("");
    assert!(resolve_api_key(None, &FireworksEnv::default()).is_none());
    assert!(resolve_api_key(Some(&empty_argument), &FireworksEnv::default()).is_none());
}

fn generated() -> SessionMetadata {
    SessionMetadata {
        litellm_session_id_generated: Some(true),
        session_id: Some("metadata-session".to_owned()),
    }
}

#[rstest]
#[case::litellm_session_first(params(Some("one"), Some("two"), None), Some("one"))]
#[case::session_second(params(None, Some("two"), None), Some("two"))]
#[case::metadata_last(params(None, None, Some(SessionMetadata { litellm_session_id_generated: None, session_id: Some("three".to_owned()) })), Some("three"))]
#[case::empty_litellm_session_skipped(params(Some(""), Some("two"), None), Some("two"))]
#[case::empty_session_skipped(params(Some(""), Some(""), Some(SessionMetadata { litellm_session_id_generated: Some(false), session_id: Some("three".to_owned()) })), Some("three"))]
#[case::generated_marker_returns_none(params(Some("one"), Some("two"), Some(generated())), None)]
#[case::empty_metadata_session_skipped(params(None, None, Some(SessionMetadata { litellm_session_id_generated: None, session_id: Some(String::new()) })), None)]
#[case::no_session_id(params(None, None, None), None)]
fn get_session_id_follows_the_litellm_order(
    #[case] params: SessionParams,
    #[case] expected: Option<&str>,
) {
    assert_eq!(get_session_id(&params).as_deref(), expected);
}

fn headers() -> Headers {
    Headers::from([(AUTHORIZATION_HEADER.to_owned(), "Bearer caller".to_owned())])
}

#[rstest]
#[case::adds_when_missing(None, Some("one"), Some("one"), 2)]
#[case::keeps_existing_exact(Some("x-session-affinity"), Some("one"), Some("explicit"), 2)]
#[case::keeps_existing_case_insensitive(
    Some("X-Session-Affinity"),
    Some("one"),
    Some("explicit"),
    2
)]
#[case::without_a_session_id(None, None, None, 1)]
fn with_session_affinity_adds_only_an_absent_header(
    #[case] existing_name: Option<&str>,
    #[case] litellm_session: Option<&str>,
    #[case] expected: Option<&str>,
    #[case] expected_headers: usize,
) {
    let mut given = headers();
    if let Some(name) = existing_name {
        given.insert(name.to_owned(), "explicit".to_owned());
    }
    let pinned = with_session_affinity(&given, &params(litellm_session, None, None));
    assert_eq!(pinned.len(), expected_headers);
    let affinity = existing_name
        .and_then(|name| pinned.get(name))
        .or_else(|| pinned.get(SESSION_AFFINITY_HEADER));
    assert_eq!(affinity.map(String::as_str), expected);
    assert_eq!(
        pinned.get(AUTHORIZATION_HEADER).map(String::as_str),
        Some("Bearer caller")
    );
}

#[test]
fn with_session_affinity_without_a_session_id_returns_the_headers_unchanged() {
    let given = headers();
    assert_eq!(
        with_session_affinity(&given, &SessionParams::default()),
        given
    );
}

#[test]
fn validate_environment_fails_without_a_key() {
    let error = validate_environment(
        &Headers::new(),
        None,
        &FireworksEnv::default(),
        &SessionParams::default(),
    )
    .unwrap_err();
    assert_eq!(error, MissingApiKey);
    assert_eq!(error.to_string(), "FIREWORKS_API_KEY is not set");
}

#[test]
fn validate_environment_builds_the_bearer_and_content_type_headers() {
    let env = env_with(None, Some("env-key"), None, None);
    let built = validate_environment(
        &Headers::new(),
        None,
        &env,
        &params(Some("session-1"), None, None),
    )
    .unwrap();
    assert_eq!(
        built.get(AUTHORIZATION_HEADER).map(String::as_str),
        Some("Bearer env-key")
    );
    assert_eq!(
        built.get(CONTENT_TYPE_HEADER).map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        built.get(SESSION_AFFINITY_HEADER).map(String::as_str),
        Some("session-1")
    );
}

#[test]
fn validate_environment_lets_a_caller_authorization_and_content_type_win() {
    let env = env_with(Some("env-key"), None, None, None);
    let given = Headers::from([
        ("Authorization".to_owned(), "Bearer caller".to_owned()),
        ("content-type".to_owned(), "multipart/form-data".to_owned()),
    ]);
    let built = validate_environment(&given, None, &env, &SessionParams::default()).unwrap();
    assert_eq!(
        built.get(AUTHORIZATION_HEADER).map(String::as_str),
        Some("Bearer caller")
    );
    assert!(!built.contains_key(CONTENT_TYPE_HEADER));
    assert_eq!(
        built.get("content-type").map(String::as_str),
        Some("multipart/form-data")
    );
}

#[test]
fn validate_environment_keeps_a_differently_cased_authorization_apart() {
    let env = env_with(Some("env-key"), None, None, None);
    let given = Headers::from([("authorization".to_owned(), "Bearer caller".to_owned())]);
    let built = validate_environment(&given, None, &env, &SessionParams::default()).unwrap();
    assert_eq!(
        built.get(AUTHORIZATION_HEADER).map(String::as_str),
        Some("Bearer env-key")
    );
    assert_eq!(
        built.get("authorization").map(String::as_str),
        Some("Bearer caller")
    );
}

#[test]
fn a_fireworks_error_keeps_its_status_message_and_headers() {
    let headers = Headers::from([("x-request-id".to_owned(), "request-1".to_owned())]);
    let error = FireworksError {
        status_code: 500,
        message: "Failed to parse response: bad".to_owned(),
        headers: Some(headers.clone()),
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(error.to_string(), "Failed to parse response: bad");
    assert_eq!(error.headers, Some(headers));
}
