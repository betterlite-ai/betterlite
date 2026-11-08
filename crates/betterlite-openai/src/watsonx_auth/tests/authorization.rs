use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::json;

use super::support::{Clock, Script, fixture, params, token_reply};
use crate::watsonx_auth::iam::DEFAULT_IAM_URL;

#[tokio::test]
async fn caller_authorization_header_is_kept_and_nothing_is_popped() {
    let clock = Clock::new();
    let client = fixture(Script::default(), &clock, &[]);
    let mut optional_params = params(&[("zen_api_key", json!("zen"))]);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer caller"));
    let resolved = client
        .validate_environment(headers, &mut optional_params, None)
        .await
        .unwrap();
    assert_eq!(
        resolved
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer caller")
    );
    assert_eq!(
        resolved
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(
        resolved.get(ACCEPT).and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(optional_params.get("zen_api_key"), Some(&json!("zen")));
}

#[tokio::test]
async fn caller_headers_override_the_two_defaults() {
    let clock = Clock::new();
    let client = fixture(Script::default(), &clock, &[]);
    let mut optional_params = params(&[]);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer caller"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    let resolved = client
        .validate_environment(headers, &mut optional_params, None)
        .await
        .unwrap();
    assert_eq!(
        resolved
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain")
    );
}

#[tokio::test]
async fn params_token_wins_over_the_zen_api_key_and_the_iam_token() {
    let clock = Clock::new();
    let client = fixture(Script::default(), &clock, &[("WX_API_KEY", "env-key")]);
    let mut optional_params = params(&[
        ("token", json!("param-token")),
        ("zen_api_key", json!("zen")),
    ]);
    let resolved = client
        .validate_environment(HeaderMap::new(), &mut optional_params, None)
        .await
        .unwrap();
    assert_eq!(
        resolved
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer param-token")
    );
    assert_eq!(optional_params.get("token"), Some(&json!("param-token")));
    assert!(!optional_params.contains_key("zen_api_key"));
    assert!(client.transport.take().is_empty());
}

#[tokio::test]
async fn environment_token_wins_over_the_zen_api_key() {
    let clock = Clock::new();
    let client = fixture(
        Script::default(),
        &clock,
        &[("WATSONX_TOKEN", "environment-token")],
    );
    let mut optional_params = params(&[("zen_api_key", json!("zen"))]);
    let resolved = client
        .validate_environment(HeaderMap::new(), &mut optional_params, None)
        .await
        .unwrap();
    assert_eq!(
        resolved
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer environment-token")
    );
    assert!(!optional_params.contains_key("zen_api_key"));
}

#[tokio::test]
async fn popped_zen_api_key_gives_a_zen_api_key_header() {
    let clock = Clock::new();
    let client = fixture(Script::default(), &clock, &[]);
    let mut optional_params = params(&[("zen_api_key", json!("zen-param"))]);
    let resolved = client
        .validate_environment(HeaderMap::new(), &mut optional_params, None)
        .await
        .unwrap();
    assert_eq!(
        resolved
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("ZenApiKey zen-param")
    );
    assert!(optional_params.is_empty());
    assert!(client.transport.take().is_empty());
}

#[tokio::test]
async fn environment_zen_api_key_gives_a_zen_api_key_header() {
    let clock = Clock::new();
    let client = fixture(
        Script::default(),
        &clock,
        &[("WATSONX_ZENAPIKEY", "zen-environment")],
    );
    let mut optional_params = params(&[]);
    let resolved = client
        .validate_environment(HeaderMap::new(), &mut optional_params, None)
        .await
        .unwrap();
    assert_eq!(
        resolved
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("ZenApiKey zen-environment")
    );
}

#[tokio::test]
async fn missing_auth_falls_back_to_the_iam_token() {
    let clock = Clock::new();
    let script = Script::default();
    script.token_route(DEFAULT_IAM_URL, vec![token_reply("iam-token", 3600)]);
    let client = fixture(script, &clock, &[("WX_API_KEY", "environment-key")]);
    let mut optional_params = params(&[]);
    let resolved = client
        .validate_environment(HeaderMap::new(), &mut optional_params, None)
        .await
        .unwrap();
    assert_eq!(
        resolved
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer iam-token")
    );
    let requests = client.transport.take();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].body,
        "grant_type=urn%3Aibm%3Aparams%3Aoauth%3Agrant-type%3Aapikey&apikey=environment-key"
    );
}
