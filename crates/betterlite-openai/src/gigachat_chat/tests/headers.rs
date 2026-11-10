#![cfg(test)]

use super::support::{Fixture, headers, secret};
use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_chat::headers::{HeaderInput, validate_environment};
use crate::openai_base_config::Headers;

async fn validated(input: &HeaderInput<'_>, fixture: &Fixture) -> Result<Headers, GigaChatError> {
    validate_environment(input, &fixture.bearer).await
}

#[tokio::test]
async fn the_token_becomes_the_authorization_header_next_to_the_json_headers() {
    let fixture = Fixture::issuing("token-1", &[]);
    let caller = headers(r#"{"X-Trace": "abc"}"#);
    let key = secret("api-key");
    let input = HeaderInput {
        api_key: Some(&key),
        credentials: None,
        fallback_key: None,
        caller_headers: &caller,
    };
    let result = validated(&input, &fixture).await.unwrap();
    assert_eq!(
        result,
        headers(
            r#"{"Accept": "application/json", "Authorization": "Bearer token-1", "Content-Type": "application/json", "X-Trace": "abc"}"#
        )
    );
    assert_eq!(
        fixture.bearer.credentials(),
        vec![Some(String::from("api-key"))]
    );
}

#[tokio::test]
async fn the_api_key_wins_over_both_environment_values() {
    let fixture = Fixture::issuing("token-1", &[]);
    let caller = Headers::new();
    let key = secret("api-key");
    let credentials = secret("credentials");
    let fallback = secret("fallback");
    let input = HeaderInput {
        api_key: Some(&key),
        credentials: Some(&credentials),
        fallback_key: Some(&fallback),
        caller_headers: &caller,
    };
    validated(&input, &fixture).await.unwrap();
    assert_eq!(
        fixture.bearer.credentials(),
        vec![Some(String::from("api-key"))]
    );
}

#[tokio::test]
async fn the_credentials_value_wins_over_the_api_key_value() {
    let fixture = Fixture::issuing("token-1", &[]);
    let caller = Headers::new();
    let credentials = secret("credentials");
    let fallback = secret("fallback");
    let input = HeaderInput {
        api_key: None,
        credentials: Some(&credentials),
        fallback_key: Some(&fallback),
        caller_headers: &caller,
    };
    validated(&input, &fixture).await.unwrap();
    assert_eq!(
        fixture.bearer.credentials(),
        vec![Some(String::from("credentials"))]
    );
}

#[tokio::test]
async fn an_empty_credential_counts_as_unset() {
    let fixture = Fixture::issuing("token-1", &[]);
    let caller = Headers::new();
    let empty = secret("");
    let fallback = secret("fallback");
    let input = HeaderInput {
        api_key: Some(&empty),
        credentials: Some(&empty),
        fallback_key: Some(&fallback),
        caller_headers: &caller,
    };
    validated(&input, &fixture).await.unwrap();
    assert_eq!(
        fixture.bearer.credentials(),
        vec![Some(String::from("fallback"))]
    );
}

#[tokio::test]
async fn without_any_credential_the_bearer_receives_none() {
    let fixture = Fixture::issuing("token-1", &[]);
    let caller = Headers::new();
    let input = HeaderInput {
        api_key: None,
        credentials: None,
        fallback_key: None,
        caller_headers: &caller,
    };
    validated(&input, &fixture).await.unwrap();
    assert_eq!(fixture.bearer.credentials(), vec![None]);
}

#[tokio::test]
async fn a_token_failure_keeps_its_status_with_no_headers() {
    let fixture = Fixture::failing(401);
    let caller = Headers::new();
    let input = HeaderInput {
        api_key: None,
        credentials: None,
        fallback_key: None,
        caller_headers: &caller,
    };
    let error = validated(&input, &fixture).await.unwrap_err();
    assert_eq!(error.status_code, 401);
    assert_eq!(error.headers, None);
}

#[tokio::test]
async fn a_caller_header_with_the_same_name_loses_to_the_configured_value() {
    let fixture = Fixture::issuing("token-1", &[]);
    let caller = headers(r#"{"Content-Type": "text/plain", "content-type": "text/plain"}"#);
    let input = HeaderInput {
        api_key: None,
        credentials: None,
        fallback_key: None,
        caller_headers: &caller,
    };
    let result = validated(&input, &fixture).await.unwrap();
    assert_eq!(
        result,
        headers(
            r#"{"Accept": "application/json", "Authorization": "Bearer token-1", "Content-Type": "application/json", "content-type": "text/plain"}"#
        )
    );
}
