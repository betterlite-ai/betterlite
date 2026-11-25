use std::sync::Arc;

use betterlite_auth::passthrough::caller_headers::error::{
    CallerHeaderError, VERTEX_MISSING_CREDENTIAL,
};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use http::HeaderMap;
use http::StatusCode;
use secrecy::SecretString;

use super::super::auth::{
    VertexAuthError, VertexAuthInput, VertexAuthPrep, prepare_vertex_auth_headers,
};
use super::support::{
    FakeExecutor, caller, capture_logs, deps, headers, json_text, prepared_text, sorted_texts,
};
use crate::pass_through_credentials::VertexPassThroughCredentials;

fn input<'request>(
    request: &'request HeaderMap,
    credentials: Option<VertexPassThroughCredentials>,
    user_api_key: &'request UserAPIKeyAuth,
) -> VertexAuthInput<'request> {
    VertexAuthInput {
        request,
        vertex_credentials: credentials,
        router_credentials: None,
        project: Some("url-project".to_owned()),
        location: Some("us-central1".to_owned()),
        user_api_key,
    }
}

async fn prepared<'request>(
    request: &'request HeaderMap,
    credentials: Option<VertexPassThroughCredentials>,
    user_api_key: &'request UserAPIKeyAuth,
    executor: Arc<FakeExecutor>,
) -> Result<VertexAuthPrep, VertexAuthError> {
    let filter_deps = deps(executor, None);
    prepare_vertex_auth_headers(input(request, credentials, user_api_key), &filter_deps).await
}

#[tokio::test]
async fn the_caller_headers_are_forwarded_with_headers_passed_through() {
    let (logs, _guard) = capture_logs();
    let executor = Arc::new(FakeExecutor::scripted(Vec::new()));
    let request = headers(&[
        ("content-type", "application/json"),
        ("x-goog-api-key", "google-key"),
        ("authorization", "Bearer caller-token"),
    ]);
    let auth = caller(None);
    let prep = prepared(&request, None, &auth, Arc::clone(&executor))
        .await
        .unwrap();
    assert!(prep.headers_passed_through);
    assert_eq!(
        sorted_texts(&prep.headers),
        vec!["authorization", "content-type", "x-goog-api-key"]
    );
    assert_eq!(
        prepared_text(&prep.headers, "x-goog-api-key"),
        Some("google-key")
    );
    assert_eq!(prep.project.as_deref(), Some("url-project"));
    assert_eq!(prep.location.as_deref(), Some("us-central1"));
    assert_eq!(executor.load_calls(), 0);
    let text = logs.text();
    assert!(
        text.contains("default_vertex_config not set, forwarding caller-provided headers"),
        "{text}"
    );
    assert!(!text.contains("google-key"), "{text}");
    assert!(!text.contains("caller-token"), "{text}");
}

#[tokio::test]
async fn a_record_without_a_project_still_forwards_the_caller_headers() {
    let (logs, _guard) = capture_logs();
    let executor = Arc::new(FakeExecutor::scripted(Vec::new()));
    let request = headers(&[("x-goog-api-key", "google-key")]);
    let auth = caller(None);
    let record = VertexPassThroughCredentials {
        vertex_project: None,
        vertex_location: Some("europe-west4".to_owned()),
        vertex_credentials: Some(json_text("credential-text")),
    };
    let prep = prepared(&request, Some(record), &auth, Arc::clone(&executor))
        .await
        .unwrap();
    assert!(prep.headers_passed_through);
    assert_eq!(sorted_texts(&prep.headers), vec!["x-goog-api-key"]);
    assert_eq!(prep.project.as_deref(), Some("url-project"));
    assert_eq!(prep.location.as_deref(), Some("us-central1"));
    assert_eq!(executor.load_calls(), 0);
    assert!(!logs.text().contains("credential-text"));
}

#[tokio::test]
async fn a_caller_without_a_google_credential_gets_the_401() {
    let executor = Arc::new(FakeExecutor::scripted(Vec::new()));
    let request = headers(&[("content-type", "application/json")]);
    let auth = caller(None);
    let error = prepared(&request, None, &auth, Arc::clone(&executor))
        .await
        .unwrap_err();
    let caller_error = match error {
        VertexAuthError::CallerHeaders(caller_error) => caller_error,
        other @ (VertexAuthError::Credentials(_)
        | VertexAuthError::CredentialsType
        | VertexAuthError::Token(_)
        | VertexAuthError::Target(_)) => {
            panic!("the caller-header error passes through unchanged: {other:?}")
        }
    };
    assert_eq!(caller_error.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(caller_error.to_string(), VERTEX_MISSING_CREDENTIAL);
    assert_eq!(executor.load_calls(), 0);
}

#[tokio::test]
async fn the_callers_own_stored_key_is_not_forwarded() {
    let master = SecretString::from("sk-master");
    let executor = Arc::new(FakeExecutor::scripted(Vec::new()));
    let request = headers(&[
        ("authorization", "Bearer stored-key"),
        ("x-goog-api-key", "google-key"),
    ]);
    let auth = caller(Some("stored-key"));
    let filter_deps = deps(Arc::clone(&executor), Some(&master));
    let prep = prepare_vertex_auth_headers(input(&request, None, &auth), &filter_deps)
        .await
        .unwrap();
    assert_eq!(sorted_texts(&prep.headers), vec!["x-goog-api-key"]);
    assert_eq!(
        prepared_text(&prep.headers, "x-goog-api-key"),
        Some("google-key")
    );
}

#[tokio::test]
async fn a_caller_missing_credential_error_variant_is_the_vertex_one() {
    let executor = Arc::new(FakeExecutor::scripted(Vec::new()));
    let request = headers(&[("x-litellm-api-key", "sk-caller")]);
    let auth = caller(Some("sk-caller"));
    let error = prepared(&request, None, &auth, executor).await.unwrap_err();
    assert!(matches!(
        error,
        VertexAuthError::CallerHeaders(CallerHeaderError::VertexCredentialMissing)
    ));
}
