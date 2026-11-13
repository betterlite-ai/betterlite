#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::CredentialError;
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, SearchProvider as _, SearchQuery,
};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::reply::{REPLY_MAX_BYTES, decode_exa_ai_search};
use super::{EXA_AI_API_BASE, ExaAiSearchConfig, ExaAiSearchError, ExaAiSearchSettings};

fn environment(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn config(values: &[(&str, &str)]) -> ExaAiSearchConfig {
    ExaAiSearchConfig {
        settings: ExaAiSearchSettings::load(&environment(values)),
    }
}

#[test]
fn exa_ai_reports_its_display_name_and_post_method() {
    assert_eq!(ExaAiSearchConfig::ui_friendly_name(), "Exa AI");
    assert_eq!(config(&[]).http_method(), HttpMethod::Post);
}

#[rstest]
#[case::unset(&[], None, EXA_AI_API_BASE)]
#[case::key(&[("EXA_API_KEY", "sk-exa")], Some("sk-exa"), EXA_AI_API_BASE)]
#[case::empty_key(&[("EXA_API_KEY", "")], None, EXA_AI_API_BASE)]
#[case::base(&[("EXA_API_BASE", "https://proxy.test")], None, "https://proxy.test")]
#[case::empty_base(&[("EXA_API_BASE", "")], None, EXA_AI_API_BASE)]
#[case::both(
    &[("EXA_API_KEY", "sk-exa"), ("EXA_API_BASE", "https://proxy.test")],
    Some("sk-exa"),
    "https://proxy.test"
)]
fn exa_ai_search_settings_load_key_and_base(
    #[case] values: &[(&str, &str)],
    #[case] expected_key: Option<&str>,
    #[case] expected_base: &str,
) {
    let settings = ExaAiSearchSettings::load(&environment(values));
    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        expected_key
    );
    assert_eq!(settings.api_base, expected_base);
}

#[test]
fn exa_ai_headers_keep_the_caller_key_and_the_content_type() {
    let config = config(&[("EXA_API_KEY", "sk-server")]);
    let mut headers = Headers::new();
    headers.insert("A".to_owned(), "b".to_owned());
    let key = SecretString::from("sk-caller");
    let validated = config
        .validated_headers(headers, Some(&key), Some("https://evil.test"))
        .unwrap();
    assert_eq!(
        validated.get("x-api-key").map(String::as_str),
        Some("sk-caller")
    );
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(validated.get("A").map(String::as_str), Some("b"));
}

#[rstest]
#[case::default_host(None, &[("EXA_API_KEY", "sk-server")], true)]
#[case::default_path(Some("https://api.exa.ai/v2"), &[("EXA_API_KEY", "sk-server")], true)]
#[case::env_host(
    Some("https://proxy.test/search"),
    &[("EXA_API_KEY", "sk-server"), ("EXA_API_BASE", "https://proxy.test")],
    true
)]
#[case::other_host(Some("https://evil.test"), &[("EXA_API_KEY", "sk-server")], false)]
#[case::other_port(Some("https://api.exa.ai:8443"), &[("EXA_API_KEY", "sk-server")], false)]
#[case::empty_base(Some(""), &[("EXA_API_KEY", "sk-server")], true)]
#[case::no_key(Some("https://evil.test"), &[], false)]
fn exa_ai_environment_key_needs_a_trusted_host(
    #[case] api_base: Option<&str>,
    #[case] values: &[(&str, &str)],
    #[case] trusted: bool,
) {
    let validated = config(values).validated_headers(Headers::new(), None, api_base);
    assert_eq!(validated.is_ok(), trusted, "{validated:?}");
}

#[test]
fn exa_ai_untrusted_base_keeps_the_server_key_out_of_the_error() {
    let config = config(&[("EXA_API_KEY", "sk-server")]);
    let error = config
        .validated_headers(Headers::new(), None, Some("https://evil.test"))
        .unwrap_err();
    assert!(matches!(
        error,
        ExaAiSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
    assert!(!error.to_string().contains("sk-server"));
}

#[test]
fn exa_ai_missing_key_reports_the_source_message() {
    let error = config(&[])
        .validated_headers(Headers::new(), None, None)
        .unwrap_err();
    assert!(matches!(error, ExaAiSearchError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "EXA_API_KEY is not set. Set `EXA_API_KEY` environment variable."
    );
}

#[test]
fn exa_ai_sign_request_fails_before_sending_without_a_key() {
    let config = config(&[]);
    let text = SearchQuery::Single(String::from("rust"));
    let request = config
        .transform_search_request(&text, &Object::new())
        .unwrap();
    let outbound = OutboundSearch {
        api_base: "https://api.exa.ai/search",
        api_key: None,
        optional_params: &Object::new(),
        body: &request,
    };
    let error = config.sign_request(Headers::new(), &outbound).unwrap_err();
    assert!(matches!(error, ExaAiSearchError::MissingApiKey));
}

#[test]
fn exa_ai_sign_request_blocks_the_server_key_for_an_untrusted_base() {
    let config = config(&[("EXA_API_KEY", "sk-server")]);
    let text = SearchQuery::Single(String::from("rust"));
    let request = config
        .transform_search_request(&text, &Object::new())
        .unwrap();
    let outbound = OutboundSearch {
        api_base: "https://evil.test/search",
        api_key: None,
        optional_params: &Object::new(),
        body: &request,
    };
    let error = config.sign_request(Headers::new(), &outbound).unwrap_err();
    assert!(matches!(
        error,
        ExaAiSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
}

#[rstest]
#[case::server_key_default_base("https://api.exa.ai/search", None, "sk-server")]
#[case::caller_key_other_base("https://evil.test/search", Some("sk-caller"), "sk-caller")]
fn exa_ai_sign_request_signs_with_the_key_and_content_type(
    #[case] api_base: &str,
    #[case] caller_key: Option<&str>,
    #[case] expected_key: &str,
) {
    let config = config(&[("EXA_API_KEY", "sk-server")]);
    let text = SearchQuery::Single(String::from("rust"));
    let request = config
        .transform_search_request(&text, &Object::new())
        .unwrap();
    let caller_secret = caller_key.map(SecretString::from);
    let outbound = OutboundSearch {
        api_base,
        api_key: caller_secret.as_ref(),
        optional_params: &Object::new(),
        body: &request,
    };
    let (headers, body) = config.sign_request(Headers::new(), &outbound).unwrap();
    assert_eq!(
        headers.get("x-api-key").map(String::as_str),
        Some(expected_key)
    );
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(body, None);
}

#[test]
fn exa_ai_validate_environment_adds_the_key_and_content_type() {
    let config = config(&[("EXA_API_KEY", "sk-server")]);
    let mut headers = Headers::new();
    headers.insert("A".to_owned(), "b".to_owned());
    let validated = config.validate_environment(headers, None, None);
    assert_eq!(
        validated.get("x-api-key").map(String::as_str),
        Some("sk-server")
    );
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(validated.get("A").map(String::as_str), Some("b"));
}

#[test]
fn exa_ai_validate_environment_leaves_headers_alone_on_a_missing_key() {
    let config = config(&[]);
    let mut headers = Headers::new();
    headers.insert("A".to_owned(), "b".to_owned());
    let kept = config.validate_environment(headers.clone(), None, None);
    assert_eq!(kept, headers);
    let validated = config.validate_environment(Headers::new(), None, None);
    assert!(validated.is_empty());
}

#[rstest]
#[case::default(None, &[], "https://api.exa.ai/search")]
#[case::caller_base(Some("https://x.test"), &[], "https://x.test/search")]
#[case::trailing_slash(Some("https://x.test/"), &[], "https://x.test//search")]
#[case::already_search(Some("https://x.test/search"), &[], "https://x.test/search")]
#[case::search_slash(Some("https://x.test/search/"), &[], "https://x.test/search//search")]
#[case::env_base(None, &[("EXA_API_BASE", "https://env.test/")], "https://env.test//search")]
#[case::empty_caller(Some(""), &[], "https://api.exa.ai/search")]
#[case::empty_env(None, &[("EXA_API_BASE", "")], "https://api.exa.ai/search")]
fn exa_ai_url_appends_search_without_trimming(
    #[case] api_base: Option<&str>,
    #[case] values: &[(&str, &str)],
    #[case] expected: &str,
) {
    let url = config(values)
        .complete_url(api_base, &Object::new(), None)
        .unwrap();
    assert_eq!(url, expected);
}

#[rstest]
#[case::plain(
    "\"search text\"",
    "{}",
    r#"{"query": "search text", "contents": {"text": true}}"#
)]
#[case::list_query(
    "[\"search\", \"text\"]",
    "{}",
    r#"{"query": "search text", "contents": {"text": true}}"#
)]
#[case::mapped(
    "\"q\"",
    r#"{"max_results": 5, "search_domain_filter": ["a.com"], "country": "US"}"#,
    r#"{"query": "q", "numResults": 5, "includeDomains": ["a.com"], "userLocation": "US", "contents": {"text": true}}"#
)]
#[case::empty_contents("\"q\"", r#"{"contents": {}}"#, r#"{"query": "q", "contents": {}}"#)]
#[case::null_contents(
    "\"q\"",
    r#"{"contents": null}"#,
    r#"{"query": "q", "contents": null}"#
)]
#[case::passthrough(
    "\"q\"",
    r#"{"numResults": 3, "query": "x", "type": "neural", "country": "GB"}"#,
    r#"{"query": "q", "userLocation": "GB", "type": "neural", "numResults": 3, "contents": {"text": true}}"#
)]
#[case::unified_wins_over_passthrough(
    "\"q\"",
    r#"{"max_results": 5, "numResults": 3, "search_domain_filter": ["a.com"], "includeDomains": ["b.com"], "country": "US", "userLocation": "GB"}"#,
    r#"{"query": "q", "numResults": 5, "includeDomains": ["a.com"], "userLocation": "US", "contents": {"text": true}}"#
)]
#[case::null_max_results(
    "\"q\"",
    r#"{"max_results": null}"#,
    r#"{"query": "q", "numResults": null, "contents": {"text": true}}"#
)]
#[case::unified_dropped(
    "\"q\"",
    r#"{"max_tokens_per_page": 5, "search_domain_filter": ["a.com"]}"#,
    r#"{"query": "q", "includeDomains": ["a.com"], "contents": {"text": true}}"#
)]
fn exa_ai_request_maps_and_passes_through(
    #[case] text: &str,
    #[case] options: &str,
    #[case] expected: &str,
) {
    let optional_params: Object = serde_json::from_str(options).unwrap();
    let query: SearchQuery = serde_json::from_str(text).unwrap();
    let request = config(&[])
        .transform_search_request(&query, &optional_params)
        .unwrap();
    let serialized = serde_json::to_string(&request).unwrap();
    let actual: IndexMap<String, Json> = serde_json::from_str(&serialized).unwrap();
    let expected_body: IndexMap<String, Json> = serde_json::from_str(expected).unwrap();
    assert_eq!(actual, expected_body);
}

#[rstest]
#[case::missing_results("{}", r#"{"results": [], "object": "search"}"#)]
#[case::empty_results(r#"{"results": []}"#, r#"{"results": [], "object": "search"}"#)]
#[case::mapped(
    r#"{"results": [{"title": "t", "url": "u", "text": "x", "publishedDate": "2024-01-02"}]}"#,
    r#"{"results": [{"title": "t", "url": "u", "snippet": "x", "date": "2024-01-02", "last_updated": null}], "object": "search"}"#
)]
#[case::missing_members(
    r#"{"results": [{}]}"#,
    r#"{"results": [{"title": "", "url": "", "snippet": "", "date": null, "last_updated": null}], "object": "search"}"#
)]
#[case::null_date(
    r#"{"results": [{"publishedDate": null}]}"#,
    r#"{"results": [{"title": "", "url": "", "snippet": "", "date": null, "last_updated": null}], "object": "search"}"#
)]
#[case::unknown_members_dropped(
    r#"{"results": [{"title": "t", "extra": 1}], "requestId": "r"}"#,
    r#"{"results": [{"title": "t", "url": "", "snippet": "", "date": null, "last_updated": null}], "object": "search"}"#
)]
fn exa_ai_reply_projects_the_result_fields(#[case] payload: &str, #[case] expected: &str) {
    let response = decode_exa_ai_search(payload.as_bytes()).unwrap();
    let serialized = serde_json::to_string(&response).unwrap();
    let actual: IndexMap<String, Json> = serde_json::from_str(&serialized).unwrap();
    let expected_reply: IndexMap<String, Json> = serde_json::from_str(expected).unwrap();
    assert_eq!(actual, expected_reply);
}

#[test]
fn exa_ai_reply_rejects_null_results() {
    let error = decode_exa_ai_search(br#"{"results": null}"#).unwrap_err();
    assert!(matches!(error, ExaAiSearchError::ResultsShape));
}

#[rstest]
#[case::null_title(r#"{"results": [{"title": null}]}"#, "title")]
#[case::null_url(r#"{"results": [{"url": null}]}"#, "url")]
#[case::null_text(r#"{"results": [{"text": null}]}"#, "text")]
fn exa_ai_reply_rejects_a_null_member(#[case] payload: &str, #[case] expected: &str) {
    let error = decode_exa_ai_search(payload.as_bytes()).unwrap_err();
    assert!(matches!(error, ExaAiSearchError::ResultField { field } if field == expected));
}

#[rstest]
#[case::string_results(r#"{"results": "abc"}"#)]
#[case::object_results(r#"{"results": {"a": 1}}"#)]
#[case::invalid_json("{")]
#[case::trailing_bytes(r#"{"results": []}{"#)]
fn exa_ai_reply_rejects_a_wrong_type(#[case] payload: &str) {
    let error = decode_exa_ai_search(payload.as_bytes()).unwrap_err();
    assert!(matches!(error, ExaAiSearchError::ReplyDecode(_)));
}

#[rstest]
#[case::numeric_title(r#"{"results": [{"title": 5}]}"#, "title")]
#[case::numeric_url(r#"{"results": [{"url": 5}]}"#, "url")]
#[case::numeric_text(r#"{"results": [{"text": 5}]}"#, "text")]
#[case::numeric_date(r#"{"results": [{"publishedDate": 5}]}"#, "publishedDate")]
fn exa_ai_reply_rejects_a_member_of_the_wrong_type(#[case] payload: &str, #[case] expected: &str) {
    let error = decode_exa_ai_search(payload.as_bytes()).unwrap_err();
    assert!(matches!(error, ExaAiSearchError::ReplyField { field, .. } if field == expected));
}

#[rstest]
#[case::null_member(r#"{"results": [null]}"#)]
#[case::string_member(r#"{"results": ["x"]}"#)]
#[case::array_member(r#"{"results": [[]]}"#)]
#[case::empty_array("[]")]
#[case::null_reply("null")]
#[case::empty_reply("")]
fn exa_ai_reply_rejects_a_value_that_is_not_an_object(#[case] payload: &str) {
    let error = decode_exa_ai_search(payload.as_bytes()).unwrap_err();
    assert!(matches!(error, ExaAiSearchError::NotAnObject));
}

#[test]
fn exa_ai_reply_rejects_a_reply_over_the_bound() {
    let oversized = vec![b' '; REPLY_MAX_BYTES + 1];
    assert!(matches!(
        decode_exa_ai_search(&oversized),
        Err(ExaAiSearchError::ReplyTooLarge { .. })
    ));
}

#[test]
fn exa_ai_reply_accepts_a_reply_at_the_bound() {
    let mut at_bound = vec![b' '; REPLY_MAX_BYTES];
    at_bound[..2].copy_from_slice(b"{}");
    let response = decode_exa_ai_search(&at_bound).unwrap();
    assert!(response.results.is_empty());
}

#[rstest]
fn replays_the_exa_ai_corpus(#[files("fuzz/corpus/exa_ai_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_exa_ai_search(&bytes));
}
