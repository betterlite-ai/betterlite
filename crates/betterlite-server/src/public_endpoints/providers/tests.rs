#![cfg(test)]

use std::sync::Arc;

use indexmap::IndexMap;
use rstest::rstest;

use super::catalog::provider_catalog;
use super::endpoints::{
    BACKUP_ASSET, build_endpoints, build_endpoints_from_json, clean_display_name,
    supported_endpoints,
};
use super::types::{ProviderSupportEntry, ProvidersFile};
use super::{get_provider_fields, get_supported_endpoints, get_supported_providers};

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
    let fields = get_provider_fields().await.0;
    let tail: Vec<&str> = fields
        .iter()
        .rev()
        .take(4)
        .rev()
        .map(|entry| entry.provider.as_str())
        .collect();
    assert_eq!(tail, ["WATSONX_TEXT", "xAI", "XINFERENCE", "CURSOR"]);
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
    let response = get_supported_endpoints().await.0;
    assert_eq!(response.endpoints[0].key, "chat_completions");
}

#[test]
fn supported_endpoints_is_cached_for_the_process_lifetime() {
    let first = supported_endpoints().expect("endpoints load");
    let second = supported_endpoints().expect("endpoints load");
    assert!(Arc::ptr_eq(&first, &second));
}
