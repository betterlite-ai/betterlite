#![cfg(test)]

use std::collections::BTreeMap;
use std::future::ready;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::BaseVectorStoreAuthCredentials;
use secrecy::SecretString;
use serde_json::json;

use super::{Authorization, auth_credentials, endpoints, get_complete_url, validate_environment};
use crate::credential_sources::getters::VertexParams;
use crate::model_urls::error::VertexUrlError;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::vertex_rag_store::error::VertexRagStoreError;

fn params<'text>(project: Option<&'text str>, location: Option<&'text str>) -> VertexParams<'text> {
    VertexParams {
        project,
        ai_project: None,
        location,
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    }
}

fn settings(project: Option<&str>, location: Option<&str>) -> VertexSettings {
    VertexSettings {
        project: project.map(str::to_owned),
        location: location.map(str::to_owned),
        credentials: None,
        adc: AdcEnvironment::default(),
    }
}

#[test]
fn the_read_and_write_routes_are_the_vertex_rag_ones() {
    let endpoints = endpoints();
    assert_eq!(
        endpoints.read,
        Some(vec![("POST".to_owned(), ":retrieveContexts".to_owned())])
    );
    assert_eq!(
        endpoints.write,
        Some(vec![("POST".to_owned(), "/ragCorpora".to_owned())])
    );
}

#[tokio::test]
async fn auth_credentials_carry_a_bearer_token_and_the_json_content_type() {
    let settings = settings(None, None);
    let credentials = auth_credentials(
        Authorization {
            params: params(None, None),
            settings: &settings,
        },
        |_credentials: Option<&ConfiguredCredentials>, _project: Option<&str>| {
            ready(Ok::<_, CredentialCacheError>(AccessToken {
                token: SecretString::from("token-abc"),
                project: "proj".to_owned(),
            }))
        },
    )
    .await
    .unwrap();
    assert!(credentials.query_params.is_none());
    let headers = credentials.headers.unwrap();
    assert_eq!(
        headers.get("Authorization"),
        Some(&json!("Bearer token-abc"))
    );
    assert_eq!(
        headers.get("Content-Type"),
        Some(&json!("application/json"))
    );
}

#[tokio::test]
async fn each_authorization_reads_its_own_token() {
    let settings = settings(None, None);
    let first = auth_credentials(
        Authorization {
            params: params(None, None),
            settings: &settings,
        },
        |_credentials: Option<&ConfiguredCredentials>, _project: Option<&str>| {
            ready(Ok::<_, CredentialCacheError>(AccessToken {
                token: SecretString::from("token-1"),
                project: "proj".to_owned(),
            }))
        },
    )
    .await
    .unwrap();
    let second = auth_credentials(
        Authorization {
            params: params(None, None),
            settings: &settings,
        },
        |_credentials: Option<&ConfiguredCredentials>, _project: Option<&str>| {
            ready(Ok::<_, CredentialCacheError>(AccessToken {
                token: SecretString::from("token-2"),
                project: "proj".to_owned(),
            }))
        },
    )
    .await
    .unwrap();
    let first_headers = first.headers.unwrap();
    let second_headers = second.headers.unwrap();
    assert_ne!(first_headers, second_headers);
    assert_eq!(
        first_headers.get("Authorization"),
        Some(&json!("Bearer token-1"))
    );
    assert_eq!(
        second_headers.get("Authorization"),
        Some(&json!("Bearer token-2"))
    );
}

#[tokio::test]
async fn a_lookup_failure_becomes_a_credentials_error() {
    let settings = settings(None, None);
    let error = auth_credentials(
        Authorization {
            params: params(None, None),
            settings: &settings,
        },
        |_credentials: Option<&ConfiguredCredentials>, _project: Option<&str>| {
            ready(Err::<AccessToken, CredentialCacheError>(
                CredentialCacheError::MissingToken,
            ))
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(error, VertexRagStoreError::Credentials(_)));
}

#[test]
fn auth_headers_overwrite_the_caller_authorization_and_content_type() {
    let headers = BTreeMap::from([
        ("Authorization".to_owned(), "Bearer caller".to_owned()),
        ("Content-Type".to_owned(), "text/plain".to_owned()),
        ("x-other".to_owned(), "keep".to_owned()),
    ]);
    let credentials = BaseVectorStoreAuthCredentials {
        headers: Some(BTreeMap::from([
            (
                "Authorization".to_owned(),
                serde_json::to_value("Bearer fresh").unwrap(),
            ),
            (
                "Content-Type".to_owned(),
                serde_json::to_value("application/json").unwrap(),
            ),
        ])),
        query_params: None,
        extra: BTreeMap::new(),
    };
    let merged = validate_environment(headers, &credentials).unwrap();
    assert_eq!(
        merged.get("Authorization").map(String::as_str),
        Some("Bearer fresh")
    );
    assert_eq!(
        merged.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(merged.get("x-other").map(String::as_str), Some("keep"));
}

#[test]
fn caller_headers_survive_without_auth_credentials() {
    let headers = BTreeMap::from([("Authorization".to_owned(), "Bearer caller".to_owned())]);
    let credentials = BaseVectorStoreAuthCredentials {
        headers: None,
        query_params: None,
        extra: BTreeMap::new(),
    };
    let merged = validate_environment(headers, &credentials).unwrap();
    assert_eq!(
        merged.get("Authorization").map(String::as_str),
        Some("Bearer caller")
    );
}

#[test]
fn a_non_text_auth_header_names_the_header() {
    let credentials = BaseVectorStoreAuthCredentials {
        headers: Some(BTreeMap::from([(
            "Authorization".to_owned(),
            serde_json::to_value(Json::Null).unwrap(),
        )])),
        query_params: None,
        extra: BTreeMap::new(),
    };
    let error = validate_environment(BTreeMap::new(), &credentials).unwrap_err();
    assert!(matches!(
        error,
        VertexRagStoreError::HeaderText(name) if name == "Authorization"
    ));
}

#[test]
fn a_complete_api_base_is_only_trimmed() {
    let settings = settings(None, None);
    let url =
        get_complete_url(Some("https://host/base///"), params(None, None), &settings).unwrap();
    assert_eq!(url, "https://host/base");
}

#[test]
fn an_empty_api_base_is_built_from_the_request_project_and_location() {
    let settings = settings(None, None);
    let url = get_complete_url(
        Some(""),
        params(Some("proj"), Some("us-central1")),
        &settings,
    )
    .unwrap();
    assert_eq!(
        url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1"
    );
}

#[test]
fn a_global_location_uses_the_global_vertex_host() {
    let settings = settings(None, None);
    let url = get_complete_url(None, params(Some("proj"), Some("global")), &settings).unwrap();
    assert_eq!(
        url,
        "https://aiplatform.googleapis.com/v1/projects/proj/locations/global"
    );
}

#[test]
fn an_absent_request_setting_falls_back_to_the_config() {
    let settings = settings(Some("proj"), Some("europe"));
    let url = get_complete_url(None, params(None, None), &settings).unwrap();
    assert_eq!(
        url,
        "https://aiplatform.europe.rep.googleapis.com/v1/projects/proj/locations/europe"
    );
}

#[test]
fn the_request_project_and_location_win_over_the_config() {
    let settings = settings(Some("configured"), Some("europe"));
    let url = get_complete_url(None, params(Some("requested"), Some("global")), &settings).unwrap();
    assert_eq!(
        url,
        "https://aiplatform.googleapis.com/v1/projects/requested/locations/global"
    );
}

#[test]
fn a_built_url_without_a_location_names_the_missing_setting() {
    let settings = settings(None, None);
    let error = get_complete_url(None, params(None, None), &settings).unwrap_err();
    assert!(matches!(
        error,
        VertexRagStoreError::Url(VertexUrlError::LocationRequired)
    ));
}

#[test]
fn a_built_url_without_a_project_names_the_missing_setting() {
    let settings = settings(None, None);
    let error = get_complete_url(None, params(None, Some("us-central1")), &settings).unwrap_err();
    assert!(matches!(
        error,
        VertexRagStoreError::Url(VertexUrlError::ProjectRequired)
    ));
}

#[test]
fn an_invalid_location_is_refused_before_it_names_a_host() {
    let settings = settings(None, None);
    let error = get_complete_url(None, params(Some("proj"), Some("US")), &settings).unwrap_err();
    assert!(matches!(
        error,
        VertexRagStoreError::Url(VertexUrlError::InvalidLocation)
    ));
}
