#![cfg(test)]

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse as _, Response};
use indexmap::IndexMap;
use rstest::rstest;
use tower::ServiceExt as _;

use super::catalog::provider_catalog;
use super::endpoints::{
    BACKUP_ASSET, build_endpoints, build_endpoints_from_json, clean_display_name,
};
use super::types::{ProviderSupportEntry, ProvidersFile, SupportedEndpointsResponse};
use super::{
    get_provider_fields, get_supported_endpoints, get_supported_providers, providers_routes,
};

async fn response_body(response: Response) -> String {
    let status = response.status();
    assert_eq!(status, StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn providers_list_is_sorted_and_includes_known_llm_providers() {
    let providers = get_supported_providers().await.0;
    let mut sorted = providers.clone();
    sorted.sort();
    assert_eq!(providers, sorted);
    assert!(providers.contains(&"openai".to_owned()));
    assert!(providers.contains(&"xai".to_owned()));
    assert!(providers.contains(&"cursor".to_owned()));
}

#[rstest]
#[case::abliteration("abliteration")]
#[case::llamagate("llamagate")]
#[case::gmi("gmi")]
#[case::sarvam("sarvam")]
#[tokio::test]
async fn providers_list_excludes_json_registry_only_slugs(#[case] excluded: &str) {
    let providers = get_supported_providers().await.0;
    assert!(!providers.contains(&excluded.to_owned()));
}

#[tokio::test]
async fn provider_fields_end_with_the_final_four_in_source_order() {
    let response = get_provider_fields().await.expect("catalog decodes");
    let body = response_body(response).await;
    let fields: Vec<super::types::ProviderCreateInfo> = serde_json::from_str(&body).unwrap();
    let tail: Vec<&str> = fields
        .iter()
        .rev()
        .take(4)
        .rev()
        .map(|entry| entry.provider.as_str())
        .collect();
    assert_eq!(tail, ["WATSONX_TEXT", "xAI", "XINFERENCE", "CURSOR"]);
}

#[tokio::test]
async fn providers_error_responds_with_litellms_unhandled_exception_body() {
    let bad_json = build_endpoints_from_json(b"not json").expect_err("not valid JSON");
    let response = bad_json.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        bytes.as_ref(),
        br#"{"error":{"message":"Internal server error","type":"internal_server_error"}}"#
    );
}

#[test]
fn provider_catalog_is_not_cached_across_calls() {
    let first = provider_catalog().expect("catalog decodes");
    let second = provider_catalog().expect("catalog decodes");
    assert_eq!(first, second);
    assert!(!first.is_empty());
}

#[test]
fn clean_display_name_strips_the_slug_suffix() {
    assert_eq!(
        clean_display_name("A2A (Agent-to-Agent) (`a2a`)"),
        "A2A (Agent-to-Agent)"
    );
}

#[test]
fn clean_display_name_can_result_in_an_empty_string() {
    assert_eq!(clean_display_name(" (`only-the-suffix`) "), "");
}

#[test]
fn clean_display_name_leaves_a_name_without_a_suffix_untouched() {
    assert_eq!(clean_display_name("OpenAI"), "OpenAI");
}

fn providers_file_with(endpoints: &[(&str, bool)]) -> ProvidersFile {
    let mut providers = IndexMap::new();
    providers.insert(
        "demo".to_owned(),
        ProviderSupportEntry {
            display_name: Some("Demo (`demo`)".to_owned()),
            endpoints: endpoints
                .iter()
                .map(|(key, supported)| ((*key).to_owned(), *supported))
                .collect(),
        },
    );
    ProvidersFile { providers }
}

#[test]
fn build_endpoints_uses_known_metadata_for_a_listed_key() {
    let raw = providers_file_with(&[("chat_completions", true)]);
    let endpoints = build_endpoints(&raw);
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].key, "chat_completions");
    assert_eq!(endpoints[0].label, "Chat Completions");
    assert_eq!(endpoints[0].endpoint, "/chat/completions");
    assert_eq!(endpoints[0].providers[0].slug, "demo");
    assert_eq!(endpoints[0].providers[0].display_name, "Demo");
}

#[test]
fn build_endpoints_auto_generates_label_and_path_for_an_unknown_key() {
    let raw = providers_file_with(&[("custom_tool_endpoint", true)]);
    let endpoints = build_endpoints(&raw);
    assert_eq!(endpoints[0].label, "Custom Tool Endpoint");
    assert_eq!(endpoints[0].endpoint, "/custom/tool/endpoint");
}

#[test]
fn build_endpoints_omits_providers_that_do_not_support_the_endpoint() {
    let raw = providers_file_with(&[("chat_completions", false)]);
    let endpoints = build_endpoints(&raw);
    assert_eq!(endpoints[0].providers.len(), 0);
}

#[test]
fn build_endpoints_preserves_first_seen_insertion_order() {
    let mut providers = IndexMap::new();
    providers.insert(
        "first".to_owned(),
        ProviderSupportEntry {
            display_name: None,
            endpoints: [
                ("responses".to_owned(), true),
                ("embeddings".to_owned(), true),
            ]
            .into_iter()
            .collect(),
        },
    );
    providers.insert(
        "second".to_owned(),
        ProviderSupportEntry {
            display_name: None,
            endpoints: [
                ("chat_completions".to_owned(), true),
                ("responses".to_owned(), true),
            ]
            .into_iter()
            .collect(),
        },
    );
    let raw = ProvidersFile { providers };
    let keys: Vec<String> = build_endpoints(&raw)
        .into_iter()
        .map(|entry| entry.key)
        .collect();
    assert_eq!(keys, ["responses", "embeddings", "chat_completions"]);
}

#[test]
fn build_endpoints_from_json_decodes_arbitrary_bytes() {
    let bytes = br#"{"providers":{"demo":{"display_name":"Demo","endpoints":{"custom_tool_endpoint":true}}}}"#;
    let endpoints = build_endpoints_from_json(bytes).expect("synthetic payload decodes");
    assert_eq!(endpoints[0].label, "Custom Tool Endpoint");
    assert_eq!(endpoints[0].endpoint, "/custom/tool/endpoint");
}

#[test]
fn load_endpoints_reads_the_bundled_backup_file() {
    let endpoints =
        build_endpoints_from_json(BACKUP_ASSET).expect("bundled endpoints file decodes");
    assert!(!endpoints.is_empty());
    assert_eq!(endpoints[0].key, "chat_completions");
    let a2a = endpoints[0]
        .providers
        .iter()
        .find(|provider| provider.slug == "a2a")
        .expect("a2a supports chat_completions");
    assert_eq!(a2a.display_name, "A2A (Agent-to-Agent)");
}

#[tokio::test]
async fn supported_endpoints_handler_matches_the_bundled_file() {
    let endpoints = build_endpoints_from_json(BACKUP_ASSET).expect("bundled file decodes");
    let state = Arc::new(Ok(SupportedEndpointsResponse { endpoints }));
    let response = get_supported_endpoints(State(state))
        .await
        .expect("endpoints load");
    let body = response_body(response).await;
    assert!(body.contains(r#""key":"chat_completions""#));
}

#[tokio::test]
async fn supported_endpoints_handler_returns_a_500_for_a_cached_decode_failure() {
    let error = build_endpoints_from_json(b"not json").expect_err("not valid JSON");
    let state = Arc::new(Err(error));
    let response = get_supported_endpoints(State(state)).await.unwrap_err();
    assert_eq!(
        response.into_response().status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn providers_routes_serves_endpoints_through_injected_router_state() {
    let request = Request::get("/public/endpoints")
        .body(Body::empty())
        .unwrap();
    let response = providers_routes().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_body(response).await;
    assert!(body.contains(r#""key":"chat_completions""#));
}
