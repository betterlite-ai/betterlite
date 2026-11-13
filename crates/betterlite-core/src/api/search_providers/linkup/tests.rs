#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::credentials::CredentialError;
use betterlite_openai::search::provider::{
    HttpMethod, ProviderResponse, SearchProvider as _, SearchQuery,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    DEFAULT_API_BASE, LinkupSearchConfig, LinkupSearchError, build_request, decode_linkup_reply,
};

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn params(pairs: &[(&str, JsonValue)]) -> Object {
    Object::from_iter(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone())),
    )
}

fn text(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}

fn config(variables: &[(&str, &str)]) -> LinkupSearchConfig {
    LinkupSearchConfig::load(None, None, &environment(variables)).unwrap()
}

fn headers(config: &LinkupSearchConfig, caller: &[(&str, &str)]) -> BTreeMap<String, String> {
    let sent: BTreeMap<String, String> = caller
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    config.validate_environment(sent, None, None)
}

#[test]
fn linkup_search_settings_require_the_key() {
    let lookup = environment(&[]);
    assert!(matches!(
        LinkupSearchConfig::load(None, None, &lookup),
        Err(LinkupSearchError::MissingApiKey)
    ));
}

#[test]
fn linkup_search_settings_read_the_key_and_base() {
    let loaded = config(&[
        ("LINKUP_API_KEY", "sk-linkup"),
        ("LINKUP_API_BASE", "https://linkup.test/v2"),
    ]);
    let sent = headers(&loaded, &[("x-trace", "t-1")]);
    assert_eq!(
        sent.get("Authorization").map(String::as_str),
        Some("Bearer sk-linkup")
    );
    assert_eq!(
        sent.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(sent.get("x-trace").map(String::as_str), Some("t-1"));
    assert_eq!(
        loaded.complete_url(None, &Object::new(), None).unwrap(),
        "https://linkup.test/v2/search"
    );
}

#[test]
fn linkup_search_settings_treat_empty_values_as_unset() {
    let lookup = environment(&[("LINKUP_API_KEY", ""), ("LINKUP_API_BASE", "")]);
    assert!(matches!(
        LinkupSearchConfig::load(None, None, &lookup),
        Err(LinkupSearchError::MissingApiKey)
    ));
    let caller = SecretString::from("sk-caller");
    let loaded = LinkupSearchConfig::load(Some(caller), None, &lookup).unwrap();
    assert_eq!(
        loaded.complete_url(None, &Object::new(), None).unwrap(),
        format!("{DEFAULT_API_BASE}/search")
    );
}

#[test]
fn linkup_search_settings_treat_an_empty_caller_base_as_unset() {
    let lookup = environment(&[("LINKUP_API_KEY", "sk-linkup")]);
    let loaded = LinkupSearchConfig::load(None, Some(""), &lookup).unwrap();
    assert_eq!(
        loaded.complete_url(Some(""), &Object::new(), None).unwrap(),
        format!("{DEFAULT_API_BASE}/search")
    );
}

#[test]
fn linkup_search_settings_refuse_an_untrusted_base() {
    let lookup = environment(&[("LINKUP_API_KEY", "sk-server")]);
    let error = LinkupSearchConfig::load(None, Some("https://evil.test"), &lookup).unwrap_err();
    assert!(matches!(
        error,
        LinkupSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
}

#[test]
fn linkup_search_settings_trust_the_configured_base() {
    let lookup = environment(&[
        ("LINKUP_API_KEY", "sk-server"),
        ("LINKUP_API_BASE", "https://linkup.test/v2"),
    ]);
    let loaded =
        LinkupSearchConfig::load(None, Some("https://linkup.test/other"), &lookup).unwrap();
    assert_eq!(
        loaded
            .complete_url(Some("https://linkup.test/other"), &Object::new(), None)
            .unwrap(),
        "https://linkup.test/other/search"
    );
}

#[test]
fn linkup_search_settings_let_a_caller_key_bypass_the_trust_check() {
    let lookup = environment(&[("LINKUP_API_KEY", "sk-server")]);
    let caller = SecretString::from("sk-caller");
    let loaded =
        LinkupSearchConfig::load(Some(caller), Some("https://evil.test"), &lookup).unwrap();
    assert_eq!(
        loaded
            .validate_environment(BTreeMap::new(), None, None)
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer sk-caller")
    );
}

#[test]
fn linkup_search_settings_keep_the_hook_key_when_one_is_passed() {
    let loaded = config(&[("LINKUP_API_KEY", "sk-server")]);
    let caller = SecretString::from("sk-hook");
    let sent = loaded.validate_environment(BTreeMap::new(), Some(&caller), None);
    assert_eq!(
        sent.get("Authorization").map(String::as_str),
        Some("Bearer sk-hook")
    );
}

#[test]
fn linkup_request_defaults_depth_and_output_type() {
    let request = build_request(&SearchQuery::Single("rust".to_owned()), &Object::new()).unwrap();
    assert_eq!(request.query, "rust");
    assert_eq!(request.depth, "standard");
    assert_eq!(request.output_type, "searchResults");
    assert_eq!(request.max_results, None);
    assert_eq!(request.include_domains, None);
    assert!(request.extra.is_empty());
}

#[test]
fn linkup_request_joins_a_query_list() {
    let query = SearchQuery::Multiple(vec!["latest".to_owned(), "rust".to_owned()]);
    let request = build_request(&query, &Object::new()).unwrap();
    assert_eq!(request.query, "latest rust");
}

#[test]
fn linkup_request_prefers_a_caller_depth_and_output_type() {
    let options = params(&[("depth", text("deep")), ("outputType", text("structured"))]);
    let request = build_request(&SearchQuery::Single("q".to_owned()), &options).unwrap();
    assert_eq!(
        (request.depth.as_str(), request.output_type.as_str()),
        ("deep", "structured")
    );
}

#[test]
fn linkup_request_defaults_a_null_depth_and_output_type() {
    let options = params(&[("depth", JsonValue::Null), ("outputType", JsonValue::Null)]);
    let request = build_request(&SearchQuery::Single("q".to_owned()), &options).unwrap();
    assert_eq!(
        (request.depth.as_str(), request.output_type.as_str()),
        ("standard", "searchResults")
    );
}

#[test]
fn linkup_request_maps_the_unified_options() {
    let options = params(&[
        ("max_results", JsonValue::from(5_i64)),
        (
            "search_domain_filter",
            JsonValue::Array(vec![text("a.test")]),
        ),
        ("country", text("us")),
        ("max_tokens_per_page", JsonValue::from(100_i64)),
    ]);
    let request = build_request(&SearchQuery::Single("q".to_owned()), &options).unwrap();
    assert_eq!(request.max_results, Some(5));
    assert_eq!(request.include_domains, Some(vec!["a.test".to_owned()]));
    assert!(request.extra.is_empty());
}

#[test]
fn linkup_request_keeps_a_mapped_value_over_a_same_named_option() {
    let options = params(&[
        ("max_results", JsonValue::from(3_i64)),
        ("maxResults", JsonValue::from(9_i64)),
        (
            "search_domain_filter",
            JsonValue::Array(vec![text("a.test")]),
        ),
        ("includeDomains", JsonValue::Array(vec![text("b.test")])),
    ]);
    let request = build_request(&SearchQuery::Single("q".to_owned()), &options).unwrap();
    assert_eq!(request.max_results, Some(3));
    assert_eq!(request.include_domains, Some(vec!["a.test".to_owned()]));
    assert!(request.extra.is_empty());
}

#[test]
fn linkup_request_passes_other_options_through() {
    let options = params(&[
        ("includeImages", JsonValue::Bool(true)),
        ("maxResults", JsonValue::from(9_i64)),
        ("q", text("ignored")),
        ("depth", text("deep")),
    ]);
    let request = build_request(&SearchQuery::Single("q".to_owned()), &options).unwrap();
    assert_eq!(request.depth, "deep");
    assert_eq!(request.max_results, None);
    assert_eq!(request.extra.len(), 2);
    assert_eq!(
        request.extra.get("includeImages"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(
        request.extra.get("maxResults"),
        Some(&JsonValue::from(9_i64))
    );
}

#[test]
fn linkup_request_error_names_the_option() {
    let options = params(&[("depth", JsonValue::from(5_i64))]);
    let error = build_request(&SearchQuery::Single("q".to_owned()), &options).unwrap_err();
    assert!(matches!(
        error,
        LinkupSearchError::OptionType { field: "depth" }
    ));
}

#[rstest]
#[case::default_base(None, "https://api.linkup.so/v1/search")]
#[case::existing_suffix(Some("https://linkup.test/v2/search"), "https://linkup.test/v2/search")]
#[case::trailing_slash(Some("https://linkup.test/v2/"), "https://linkup.test/v2//search")]
fn linkup_url_appends_the_search_suffix(#[case] base: Option<&str>, #[case] expected: &str) {
    let loaded = config(&[("LINKUP_API_KEY", "sk-linkup")]);
    assert_eq!(
        loaded.complete_url(base, &Object::new(), None).unwrap(),
        expected
    );
}

#[test]
fn linkup_reply_maps_text_items() {
    let body = br#"{"results":[{"name":"Rust","url":"https://rust.test","content":"safe"}]}"#;
    let response = decode_linkup_reply(body).unwrap();
    assert_eq!(response.object.0, "search");
    assert_eq!(response.results.len(), 1);
    let result = &response.results[0];
    assert_eq!(
        (
            result.title.as_str(),
            result.url.as_str(),
            result.snippet.as_str()
        ),
        ("Rust", "https://rust.test", "safe")
    );
    assert_eq!(
        (result.date.as_deref(), result.last_updated.as_deref()),
        (None, None)
    );
}

#[test]
fn linkup_reply_falls_back_to_the_url_for_an_image_title() {
    let body = br#"{"results":[{"type":"image","url":"https://img.test/a.png","content":"a"}]}"#;
    let response = decode_linkup_reply(body).unwrap();
    assert_eq!(response.results[0].title, "https://img.test/a.png");
    assert_eq!(response.results[0].url, "https://img.test/a.png");
}

#[test]
fn linkup_reply_keeps_a_null_image_name_as_the_url() {
    let body = br#"{"results":[{"type":"image","name":null,"url":"https://img.test/a.png"}]}"#;
    let response = decode_linkup_reply(body).unwrap();
    assert_eq!(response.results[0].title, "https://img.test/a.png");
}

#[test]
fn linkup_reply_skips_unknown_item_types() {
    let body = br#"{"results":[
        {"name":"Text","url":"https://a.test","content":"a"},
        {"type":"video","name":"Clip","url":"https://b.test"},
        {"name":"Other","url":"https://c.test","content":"c"}
    ]}"#;
    let response = decode_linkup_reply(body).unwrap();
    let urls: Vec<&str> = response
        .results
        .iter()
        .map(|result| result.url.as_str())
        .collect();
    assert_eq!(urls, ["https://a.test", "https://c.test"]);
}

#[rstest]
#[case::sourced_answer(br#"{"answer":"text","sources":[]}"#)]
#[case::empty_results(br#"{"results":[]}"#)]
fn linkup_reply_without_results_is_empty(#[case] body: &[u8]) {
    let response = decode_linkup_reply(body).unwrap();
    assert!(response.results.is_empty());
}

#[test]
fn linkup_reply_rejects_malformed_json() {
    assert!(matches!(
        decode_linkup_reply(b"not json"),
        Err(LinkupSearchError::ReplyJson { .. })
    ));
}

#[test]
fn linkup_config_reports_its_display_name_and_method() {
    assert_eq!(LinkupSearchConfig::ui_friendly_name(), "Linkup");
    let loaded = config(&[("LINKUP_API_KEY", "sk-linkup")]);
    assert_eq!(loaded.http_method(), HttpMethod::Post);
    assert!(!loaded.supports_rich_search_input());
}

#[test]
fn linkup_response_hook_decodes_the_provider_reply() {
    let loaded = config(&[("LINKUP_API_KEY", "sk-linkup")]);
    let headers = BTreeMap::new();
    let response = ProviderResponse {
        status_code: 200,
        headers: &headers,
        text: r#"{"results":[{"name":"Rust","url":"https://rust.test"}]}"#,
    };
    let decoded = loaded.transform_search_response(&response).unwrap();
    assert_eq!(decoded.results[0].title, "Rust");
}

#[test]
fn linkup_key_never_reaches_debug_output() {
    let loaded = config(&[("LINKUP_API_KEY", "sk-secret")]);
    let rendered = format!("{loaded:?}");
    assert!(!rendered.contains("sk-secret"));
    assert!(!loaded.api_key.expose_secret().is_empty());
}

#[rstest]
fn replays_the_linkup_corpus(#[files("fuzz/corpus/linkup_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(response) = decode_linkup_reply(&bytes) {
        assert_eq!(str::from_utf8(&bytes).map(str::len), Ok(bytes.len()));
        drop(response.results);
    }
}
