use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_vertex::endpoint_target::error::EndpointTargetError;
use betterlite_vertex::endpoint_target::target::AuthHeader;
use betterlite_vertex::token_cache::error::CredentialCacheError;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::auth::{
    VertexAuthError, VertexAuthInput, VertexAuthPrep, prepare_vertex_auth_headers,
    vertex_token_target,
};
use super::super::override_params::RouterCredentials;
use super::support::{
    FakeExecutor, LoadRequest, SeenCredentials, caller, capture_logs, deps, headers, json_text,
    loaded, parameters, prepared_text, sorted_texts,
};
use crate::pass_through_credentials::VertexPassThroughCredentials;

const GENERATE_CONTENT: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/us-central1/publishers/google/models/:generateContent";

struct Setup {
    credentials: Option<VertexPassThroughCredentials>,
    router: Option<RouterCredentials>,
    project: Option<String>,
    location: Option<String>,
}

fn request_headers() -> http::HeaderMap {
    headers(&[
        ("anthropic-beta", "beta-1"),
        ("content-type", "application/json"),
        ("x-goog-api-key", "caller-google-key"),
    ])
}

async fn prepared(
    setup: &Setup,
    executor: Arc<FakeExecutor>,
) -> Result<VertexAuthPrep, VertexAuthError> {
    let request = request_headers();
    let auth = caller(None);
    let filter_deps = deps(executor, None);
    let input = VertexAuthInput {
        request: &request,
        vertex_credentials: setup.credentials.clone(),
        router_credentials: setup.router.as_ref(),
        project: setup.project.clone(),
        location: setup.location.clone(),
        user_api_key: &auth,
    };
    prepare_vertex_auth_headers(input, &filter_deps).await
}

fn configured(project: &str, location: Option<&str>, credentials: Option<Json>) -> Setup {
    Setup {
        credentials: Some(VertexPassThroughCredentials {
            vertex_project: Some(project.to_owned()),
            vertex_location: location.map(str::to_owned),
            vertex_credentials: credentials,
        }),
        router: None,
        project: Some("url-project".to_owned()),
        location: Some("url-location".to_owned()),
    }
}

#[tokio::test]
async fn configured_credentials_supply_the_project_and_token() {
    let (logs, _guard) = capture_logs();
    let executor = Arc::new(FakeExecutor::scripted(vec![Ok(loaded(
        "tok-1",
        Some("cfg-project"),
    ))]));
    let setup = configured(
        "cfg-project",
        Some("europe-west4"),
        Some(json_text("credential-text")),
    );
    let prep = prepared(&setup, Arc::clone(&executor)).await.unwrap();
    assert!(!prep.headers_passed_through);
    assert_eq!(
        sorted_texts(&prep.headers),
        vec!["Authorization", "anthropic-beta", "content-type"]
    );
    assert_eq!(
        prepared_text(&prep.headers, "Authorization"),
        Some("Bearer tok-1")
    );
    assert_eq!(prep.project.as_deref(), Some("cfg-project"));
    assert_eq!(prep.location.as_deref(), Some("europe-west4"));
    assert_eq!(
        executor.requests(),
        vec![LoadRequest {
            credentials: SeenCredentials::Text("credential-text".to_owned()),
            project: Some("cfg-project".to_owned()),
        }]
    );
    let text = logs.text();
    assert!(!text.contains("tok-1"), "{text}");
    assert!(!text.contains("credential-text"), "{text}");
    assert!(!text.contains("caller-google-key"), "{text}");
}

#[tokio::test]
async fn a_configured_project_without_a_location_keeps_the_url_location() {
    let executor = Arc::new(FakeExecutor::scripted(vec![Ok(loaded("tok-2", None))]));
    let setup = configured("cfg-project", None, None);
    let prep = prepared(&setup, Arc::clone(&executor)).await.unwrap();
    assert_eq!(prep.project.as_deref(), Some("cfg-project"));
    assert_eq!(prep.location.as_deref(), Some("url-location"));
    assert_eq!(
        executor.requests(),
        vec![LoadRequest {
            credentials: SeenCredentials::None,
            project: Some("cfg-project".to_owned()),
        }]
    );
}

#[tokio::test]
async fn router_credentials_use_default_credentials_and_the_token_project() {
    let (logs, _guard) = capture_logs();
    let executor = Arc::new(FakeExecutor::scripted(vec![Ok(loaded(
        "tok-3",
        Some("adc-project"),
    ))]));
    let setup = Setup {
        credentials: Some(VertexPassThroughCredentials {
            vertex_project: Some("cfg-project".to_owned()),
            vertex_location: Some("europe-west4".to_owned()),
            vertex_credentials: Some(json_text("credential-text")),
        }),
        router: Some(RouterCredentials {
            litellm_params: Some(parameters(&[(
                "vertex_project",
                json_text("store-project"),
            )])),
        }),
        project: None,
        location: Some("us-central1".to_owned()),
    };
    let prep = prepared(&setup, Arc::clone(&executor)).await.unwrap();
    assert_eq!(prep.project.as_deref(), Some("adc-project"));
    assert_eq!(prep.location.as_deref(), Some("us-central1"));
    assert_eq!(
        prepared_text(&prep.headers, "Authorization"),
        Some("Bearer tok-3")
    );
    assert_eq!(
        executor.requests(),
        vec![LoadRequest {
            credentials: SeenCredentials::None,
            project: None,
        }]
    );
    assert!(!logs.text().contains("credential-text"));
}

#[tokio::test]
async fn every_call_starts_from_an_empty_token_state() {
    let executor = Arc::new(FakeExecutor::scripted(vec![
        Ok(loaded("tok-a", Some("p"))),
        Ok(loaded("tok-b", Some("p"))),
    ]));
    let setup = Setup {
        credentials: None,
        router: Some(RouterCredentials {
            litellm_params: None,
        }),
        project: Some("p".to_owned()),
        location: Some("us-central1".to_owned()),
    };
    let first = prepared(&setup, Arc::clone(&executor)).await.unwrap();
    let second = prepared(&setup, Arc::clone(&executor)).await.unwrap();
    assert_eq!(
        prepared_text(&first.headers, "Authorization"),
        Some("Bearer tok-a")
    );
    assert_eq!(
        prepared_text(&second.headers, "Authorization"),
        Some("Bearer tok-b")
    );
    assert_eq!(executor.load_calls(), 2);
}

#[tokio::test]
async fn an_invalid_location_fails_the_target_without_leaking_the_token() {
    let executor = Arc::new(FakeExecutor::scripted(vec![Ok(loaded("tok-4", Some("p")))]));
    let setup = configured("p", Some("BAD LOCATION"), None);
    let error = prepared(&setup, executor).await.unwrap_err();
    assert!(matches!(&error, VertexAuthError::Target(_)));
    let text = error.to_string();
    assert!(text.contains("invalid vertex_location format"), "{text}");
    assert!(!text.contains("tok-4"), "{text}");
}

#[tokio::test]
async fn a_token_load_error_is_returned_without_the_credential_text() {
    let executor = Arc::new(FakeExecutor::scripted(vec![Err(
        CredentialCacheError::MissingCredentials,
    )]));
    let setup = configured("cfg-project", None, Some(json_text("credential-text")));
    let error = prepared(&setup, executor).await.unwrap_err();
    assert!(matches!(
        &error,
        VertexAuthError::Token(CredentialCacheError::MissingCredentials)
    ));
    assert!(!error.to_string().contains("credential-text"));
}

#[tokio::test]
async fn a_null_credential_value_is_the_default_credentials() {
    let executor = Arc::new(FakeExecutor::scripted(vec![Ok(loaded(
        "tok-null",
        Some("cfg-project"),
    ))]));
    let setup = configured("cfg-project", None, Some(Json::Null));
    let prep = prepared(&setup, Arc::clone(&executor)).await.unwrap();
    assert_eq!(prep.project.as_deref(), Some("cfg-project"));
    assert_eq!(
        executor.requests(),
        vec![LoadRequest {
            credentials: SeenCredentials::None,
            project: Some("cfg-project".to_owned()),
        }]
    );
}

#[tokio::test]
async fn an_object_credential_is_passed_on_as_its_json_text() {
    let executor = Arc::new(FakeExecutor::scripted(Vec::new()));
    let setup = configured(
        "cfg-project",
        None,
        Some(Json::Object(parameters(&[(
            "type",
            json_text("service_account"),
        )]))),
    );
    let error = prepared(&setup, executor).await.unwrap_err();
    assert!(matches!(&error, VertexAuthError::Credentials(_)));
    assert!(
        error.to_string().contains("not a credential document"),
        "{error}"
    );
}

#[tokio::test]
async fn a_malformed_credential_document_is_a_source_error() {
    let executor = Arc::new(FakeExecutor::scripted(Vec::new()));
    let setup = configured(
        "cfg-project",
        None,
        Some(json_text("{\"type\": \"service_account\"}")),
    );
    let error = prepared(&setup, Arc::clone(&executor)).await.unwrap_err();
    assert!(matches!(&error, VertexAuthError::Credentials(_)));
    let text = error.to_string();
    assert!(
        text.contains("Unable to load vertex credentials from environment"),
        "{text}"
    );
    assert!(text.contains("not a credential document"), "{text}");
    assert_eq!(executor.load_calls(), 0);
}

#[tokio::test]
async fn a_credentials_value_that_is_not_text_or_an_object_is_an_error() {
    let executor = Arc::new(FakeExecutor::scripted(Vec::new()));
    let setup = configured("cfg-project", None, Some(Json::Bool(true)));
    let error = prepared(&setup, executor).await.unwrap_err();
    assert!(matches!(&error, VertexAuthError::CredentialsType));
    assert_eq!(
        error.to_string(),
        "vertex_credentials must be a string or an object"
    );
}

#[test]
fn the_token_target_is_the_generate_content_route() {
    let token = SecretString::from("tok-direct");
    let target = vertex_token_target(&token, Some("p"), Some("us-central1")).unwrap();
    assert_eq!(target.url, GENERATE_CONTENT);
    let returned = match target.auth {
        Some(AuthHeader::Token(returned)) => returned,
        Some(AuthHeader::Headers(_)) | None => panic!("the passed token is the auth header"),
    };
    assert_eq!(returned.expose_secret(), "tok-direct");
}

#[test]
fn a_single_word_location_uses_the_rep_host_for_the_target() {
    let target = vertex_token_target(&SecretString::from("tok"), Some("p"), Some("us")).unwrap();
    assert_eq!(
        target.url,
        "https://aiplatform.us.rep.googleapis.com/v1/projects/p/locations/us/publishers/google/models/:generateContent"
    );
}

#[test]
fn the_global_location_uses_the_global_host_for_the_target() {
    let target =
        vertex_token_target(&SecretString::from("tok"), Some("p"), Some("global")).unwrap();
    assert_eq!(
        target.url,
        "https://aiplatform.googleapis.com/v1/projects/p/locations/global/publishers/google/models/:generateContent"
    );
}

#[test]
fn a_missing_location_uses_the_default_region() {
    let target = vertex_token_target(&SecretString::from("tok"), Some("p"), None).unwrap();
    assert_eq!(target.url, GENERATE_CONTENT);
}

#[test]
fn a_missing_project_is_an_error() {
    let target = vertex_token_target(&SecretString::from("tok"), None, Some("us-central1"));
    assert!(matches!(target, Err(EndpointTargetError::VertexUrl(_))));
}
