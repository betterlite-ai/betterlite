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
use crate::vertex_discovery_search::error::VertexDiscoverySearchError;
use crate::vertex_discovery_search::options::{DEFAULT_COLLECTION_ID, VertexSearchOptions};

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

fn options(collection: &str, engine: Option<&str>, store: Option<&str>) -> VertexSearchOptions {
    VertexSearchOptions {
        vertex_collection_id: collection.to_owned(),
        vertex_engine_id: engine.map(str::to_owned),
        vector_store_id: store.map(str::to_owned),
    }
}

#[test]
fn the_search_read_route_is_a_post_and_the_write_map_is_empty() {
    let endpoints = endpoints();
    assert_eq!(
        endpoints.read,
        Some(vec![("POST".to_owned(), ":search".to_owned())])
    );
    assert_eq!(endpoints.write, Some(Vec::new()));
}

#[tokio::test]
async fn a_discovery_search_auth_carries_a_fresh_bearer_token() {
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
async fn the_lookup_receives_the_request_project() {
    let settings = settings(Some("configured"), None);
    let credentials = auth_credentials(
        Authorization {
            params: params(Some("requested"), None),
            settings: &settings,
        },
        |_credentials: Option<&ConfiguredCredentials>, project: Option<&str>| {
            ready(Ok::<_, CredentialCacheError>(AccessToken {
                token: SecretString::from(project.unwrap_or("missing")),
                project: project.unwrap_or("missing").to_owned(),
            }))
        },
    )
    .await
    .unwrap();
    assert_eq!(
        credentials.headers.unwrap().get("Authorization"),
        Some(&json!("Bearer requested"))
    );
}

#[tokio::test]
async fn each_search_reads_its_own_token() {
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
    assert_ne!(first.headers.unwrap(), second.headers.unwrap());
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
    assert!(matches!(error, VertexDiscoverySearchError::Credentials(_)));
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
        VertexDiscoverySearchError::HeaderText(name) if name == "Authorization"
    ));
}

#[test]
fn an_api_base_is_only_trimmed() {
    let settings = settings(None, None);
    let url = get_complete_url(
        Some("https://host/v1///"),
        params(None, None),
        &settings,
        &VertexSearchOptions::default(),
    )
    .unwrap();
    assert_eq!(url, "https://host/v1");
}

#[test]
fn an_engine_target_uses_the_default_serving_config() {
    let settings = settings(None, None);
    let url = get_complete_url(
        None,
        params(Some("proj"), Some("us-central1")),
        &settings,
        &options(DEFAULT_COLLECTION_ID, Some("app"), Some("ignored")),
    )
    .unwrap();
    assert_eq!(
        url,
        "https://discoveryengine.googleapis.com/v1/projects/proj/locations/us-central1/collections/default_collection/engines/app/servingConfigs/default_serving_config"
    );
}

#[test]
fn a_datastore_target_uses_its_default_config() {
    let settings = settings(None, None);
    let url = get_complete_url(
        None,
        params(Some("proj"), Some("us-central1")),
        &settings,
        &options("collection", None, Some("store")),
    )
    .unwrap();
    assert_eq!(
        url,
        "https://discoveryengine.googleapis.com/v1/projects/proj/locations/us-central1/collections/collection/dataStores/store/servingConfigs/default_config"
    );
}

#[test]
fn the_collection_and_target_segments_are_encoded() {
    let settings = settings(None, None);
    let url = get_complete_url(
        None,
        params(Some("proj"), Some("us-central1")),
        &settings,
        &options("my collection", Some("apps/1"), None),
    )
    .unwrap();
    assert!(url.contains("/collections/my%20collection/"), "{url}");
    assert!(url.contains("/engines/apps%2F1/"), "{url}");
}

#[test]
fn a_datastore_target_without_a_store_names_the_missing_field() {
    let settings = settings(None, None);
    let error = get_complete_url(
        None,
        params(Some("proj"), Some("us-central1")),
        &settings,
        &options(DEFAULT_COLLECTION_ID, None, None),
    )
    .unwrap_err();
    assert!(matches!(error, VertexDiscoverySearchError::StoreRequired));
    assert_eq!(
        error.to_string(),
        "vector_store_id is required when vertex_engine_id is not set"
    );
}

#[test]
fn a_built_url_without_a_location_names_the_missing_setting() {
    let settings = settings(None, None);
    let error = get_complete_url(
        None,
        params(Some("proj"), None),
        &settings,
        &options(DEFAULT_COLLECTION_ID, None, Some("store")),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        VertexDiscoverySearchError::Url(VertexUrlError::LocationRequired)
    ));
}

#[test]
fn a_built_url_without_a_project_names_the_missing_setting() {
    let settings = settings(None, None);
    let error = get_complete_url(
        None,
        params(None, Some("us-central1")),
        &settings,
        &options(DEFAULT_COLLECTION_ID, None, Some("store")),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        VertexDiscoverySearchError::Url(VertexUrlError::ProjectRequired)
    ));
}

#[test]
fn the_configured_project_and_location_are_used_when_the_request_has_none() {
    let settings = settings(Some("configured"), Some("europe"));
    let url = get_complete_url(
        None,
        params(None, None),
        &settings,
        &options(DEFAULT_COLLECTION_ID, None, Some("store")),
    )
    .unwrap();
    assert!(
        url.contains("/projects/configured/locations/europe/"),
        "{url}"
    );
}

#[test]
fn an_empty_collection_is_refused_as_a_segment() {
    let settings = settings(None, None);
    let error = get_complete_url(
        None,
        params(Some("proj"), Some("us-central1")),
        &settings,
        &options("", None, Some("store")),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        VertexDiscoverySearchError::Segment("vertex_collection_id", _)
    ));
}
