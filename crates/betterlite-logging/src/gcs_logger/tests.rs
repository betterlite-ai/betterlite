#![cfg(test)]

use std::fs;

use secrecy::{ExposeSecret as _, SecretString};

use crate::gcs_client::gcs_storage_auth::STORAGE_ENDPOINT;
use crate::gcs_mock::auth::{
    EnsureAccessToken, GetTokenAndUrl, VertexAuth as _, VertexAuthProvider,
};

use super::auth::{AuthError, GoogleCloudAuth};

fn token_request(credentials: Option<&str>) -> EnsureAccessToken {
    EnsureAccessToken {
        credentials: credentials.map(|text| SecretString::from(text.to_owned())),
        project_id: None,
        custom_llm_provider: VertexAuthProvider::VertexAi,
    }
}

fn url_request(auth_header: Option<SecretString>) -> GetTokenAndUrl {
    GetTokenAndUrl {
        model: "gcs-bucket".to_owned(),
        auth_header,
        vertex_credentials: None,
        vertex_project: None,
        vertex_location: None,
        gemini_api_key: None,
        stream: None,
        custom_llm_provider: VertexAuthProvider::VertexAi,
        api_base: None,
    }
}

#[test]
fn the_synchronous_token_path_is_refused() {
    let error = GoogleCloudAuth::default()
        .ensure_access_token(token_request(None))
        .expect_err("the logging backend is asynchronous");
    assert!(matches!(error, AuthError::Synchronous));
}

#[test]
fn the_token_and_url_carry_the_storage_endpoint() {
    let result = GoogleCloudAuth::default().get_token_and_url(url_request(Some(
        SecretString::from("token-from-the-request"),
    )));
    let token_and_url = result.expect("the request carries a token");
    assert_eq!(
        token_and_url.token.expose_secret(),
        "token-from-the-request"
    );
    assert_eq!(token_and_url.url, STORAGE_ENDPOINT);
}

#[test]
fn a_missing_token_is_refused() {
    let error = GoogleCloudAuth::default()
        .get_token_and_url(url_request(None))
        .expect_err("the request carries no token");
    assert!(matches!(error, AuthError::MissingToken));
}

#[tokio::test]
async fn an_unsupported_credential_type_is_refused() {
    let document = r#"{
        "type": "external_account",
        "audience": "the-audience",
        "subject_token_type": "urn:ietf:params:oauth:token-type:jwt",
        "credential_source": {"url": "https://example.com/token"}
    }"#;
    let error = GoogleCloudAuth::default()
        .ensure_access_token_async(token_request(Some(document)))
        .await
        .expect_err("the credential type cannot be built");
    assert!(matches!(error, AuthError::Unsupported { kind } if kind == "external_account"));
}

#[tokio::test]
async fn a_credentials_file_is_read_from_its_path() {
    let file = tempfile::NamedTempFile::new().expect("the temporary credentials file is created");
    fs::write(file.path(), r#"{"type": "impersonated_service_account"}"#)
        .expect("the credentials document is written");
    let path = file.path().to_str().expect("the temporary path is UTF-8");
    let error = GoogleCloudAuth::default()
        .ensure_access_token_async(token_request(Some(path)))
        .await
        .expect_err("the credential type cannot be built");
    assert!(
        matches!(error, AuthError::Unsupported { kind } if kind == "impersonated_service_account")
    );
}

#[tokio::test]
async fn a_credentials_document_without_a_type_is_refused() {
    let error = GoogleCloudAuth::default()
        .ensure_access_token_async(token_request(Some(r#"{"project_id": "demo"}"#)))
        .await
        .expect_err("the document has no credential type");
    assert!(matches!(error, AuthError::Decode(_)));
}

#[tokio::test]
async fn malformed_credentials_json_is_refused() {
    let error = GoogleCloudAuth::default()
        .ensure_access_token_async(token_request(Some("not a document")))
        .await
        .expect_err("the document is not JSON");
    assert!(matches!(error, AuthError::Document));
}
