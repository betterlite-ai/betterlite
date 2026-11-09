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

use super::reply::{REPLY_MAX_BYTES, decode_serper_search};
use super::{SERPER_API_BASE, SerperSearchConfig, SerperSearchError, SerperSearchSettings};

fn environment(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn config(values: &[(&str, &str)]) -> SerperSearchConfig {
    SerperSearchConfig {
        settings: SerperSearchSettings::load(&environment(values)),
    }
}

#[test]
fn serper_reports_its_display_name_and_post_method() {
    assert_eq!(SerperSearchConfig::ui_friendly_name(), "Serper");
    assert_eq!(config(&[]).http_method(), HttpMethod::Post);
}

#[rstest]
#[case::unset(&[], None, SERPER_API_BASE)]
#[case::key(&[("SERPER_API_KEY", "sk-serper")], Some("sk-serper"), SERPER_API_BASE)]
#[case::empty_key(&[("SERPER_API_KEY", "")], None, SERPER_API_BASE)]
#[case::base(&[("SERPER_API_BASE", "https://proxy.test")], None, "https://proxy.test")]
#[case::empty_base(&[("SERPER_API_BASE", "")], None, SERPER_API_BASE)]
#[case::both(
    &[("SERPER_API_KEY", "sk-serper"), ("SERPER_API_BASE", "https://proxy.test")],
    Some("sk-serper"),
    "https://proxy.test"
)]
fn serper_search_settings_load_key_and_base(
    #[case] values: &[(&str, &str)],
    #[case] expected_key: Option<&str>,
    #[case] expected_base: &str,
) {
    let settings = SerperSearchSettings::load(&environment(values));
    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        expected_key
    );
    assert_eq!(settings.api_base, expected_base);
}

#[test]
fn serper_headers_keep_the_caller_key_and_the_content_type() {
    let config = config(&[("SERPER_API_KEY", "sk-server")]);
    let mut headers = Headers::new();
    headers.insert("A".to_owned(), "b".to_owned());
    let key = SecretString::from("sk-caller");
    let validated = config
        .validated_headers(headers, Some(&key), Some("https://evil.test"))
        .unwrap();
    assert_eq!(
        validated.get("X-API-KEY").map(String::as_str),
        Some("sk-caller")
    );
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(validated.get("A").map(String::as_str), Some("b"));
}

#[rstest]
#[case::default_host(None, &[("SERPER_API_KEY", "sk-server")], true)]
#[case::default_path(Some("https://google.serper.dev/search"), &[("SERPER_API_KEY", "sk-server")], true)]
#[case::env_host(
    Some("https://proxy.test/search"),
    &[("SERPER_API_KEY", "sk-server"), ("SERPER_API_BASE", "https://proxy.test")],
    true
)]
#[case::other_host(Some("https://evil.test"), &[("SERPER_API_KEY", "sk-server")], false)]
#[case::other_port(Some("https://google.serper.dev:8443"), &[("SERPER_API_KEY", "sk-server")], false)]
#[case::empty_base(Some(""), &[("SERPER_API_KEY", "sk-server")], true)]
#[case::no_key(Some("https://evil.test"), &[], false)]
fn serper_environment_key_needs_a_trusted_host(
    #[case] api_base: Option<&str>,
    #[case] values: &[(&str, &str)],
    #[case] trusted: bool,
) {
    let validated = config(values).validated_headers(Headers::new(), None, api_base);
    assert_eq!(validated.is_ok(), trusted, "{validated:?}");
}

#[test]
fn serper_untrusted_base_keeps_the_server_key_out_of_the_error() {
    let config = config(&[("SERPER_API_KEY", "sk-server")]);
    let error = config
        .validated_headers(Headers::new(), None, Some("https://evil.test"))
        .unwrap_err();
    assert!(matches!(
        error,
        SerperSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
    assert!(!error.to_string().contains("sk-server"));
}

#[test]
fn serper_missing_key_reports_the_source_message() {
    let error = config(&[])
        .validated_headers(Headers::new(), None, None)
        .unwrap_err();
    assert!(matches!(error, SerperSearchError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "SERPER_API_KEY is not set. Set `SERPER_API_KEY` environment variable."
    );
}

#[test]
fn serper_sign_request_fails_before_sending_without_a_key() {
    let config = config(&[]);
    let text = SearchQuery::Single(String::from("rust"));
    let request = config
        .transform_search_request(&text, &Object::new())
        .unwrap();
    let outbound = OutboundSearch {
        api_base: "https://google.serper.dev/search",
        api_key: None,
        optional_params: &Object::new(),
        body: &request,
    };
    let error = config.sign_request(Headers::new(), &outbound).unwrap_err();
    assert!(matches!(error, SerperSearchError::MissingApiKey));
}

#[test]
fn serper_sign_request_blocks_the_server_key_for_an_untrusted_base() {
    let config = config(&[("SERPER_API_KEY", "sk-server")]);
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
        SerperSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
}

#[rstest]
#[case::server_key_default_base("https://google.serper.dev/search", None, "sk-server")]
#[case::caller_key_other_base("https://evil.test/search", Some("sk-caller"), "sk-caller")]
fn serper_sign_request_signs_with_the_key_and_content_type(
    #[case] api_base: &str,
    #[case] caller_key: Option<&str>,
    #[case] expected_key: &str,
) {
    let config = config(&[("SERPER_API_KEY", "sk-server")]);
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
        headers.get("X-API-KEY").map(String::as_str),
        Some(expected_key)
    );
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(body, None);
}

#[test]
fn serper_validate_environment_adds_the_key_and_content_type() {
    let config = config(&[("SERPER_API_KEY", "sk-server")]);
    let mut headers = Headers::new();
    headers.insert("A".to_owned(), "b".to_owned());
    let validated = config.validate_environment(headers, None, None);
    assert_eq!(
        validated.get("X-API-KEY").map(String::as_str),
        Some("sk-server")
    );
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(validated.get("A").map(String::as_str), Some("b"));
}

#[test]
fn serper_validate_environment_leaves_headers_alone_on_a_missing_key() {
    let config = config(&[]);
    let mut headers = Headers::new();
    headers.insert("A".to_owned(), "b".to_owned());
    let kept = config.validate_environment(headers.clone(), None, None);
    assert_eq!(kept, headers);
}

#[rstest]
#[case::default(None, &[], "https://google.serper.dev/search")]
#[case::caller_base(Some("https://x.test"), &[], "https://x.test/search")]
#[case::trailing_slash(Some("https://x.test/"), &[], "https://x.test/search")]
#[case::double_slash(Some("https://x.test//"), &[], "https://x.test/search")]
#[case::already_search(Some("https://x.test/search"), &[], "https://x.test/search")]
#[case::search_slash(Some("https://x.test/search/"), &[], "https://x.test/search")]
#[case::bare_slash(Some("/"), &[], "/search")]
#[case::env_base(None, &[("SERPER_API_BASE", "https://env.test/")], "https://env.test/search")]
#[case::empty_caller(Some(""), &[], "https://google.serper.dev/search")]
#[case::empty_env(None, &[("SERPER_API_BASE", "")], "https://google.serper.dev/search")]
fn serper_url_trims_trailing_slashes(
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
#[case::plain("\"q\"", "{}", r#"{"q": "q"}"#)]
#[case::list_query("[\"a\", \"b\"]", "{}", r#"{"q": "a b"}"#)]
#[case::mapped(
    "\"q\"",
    r#"{"max_results": 5, "country": "US"}"#,
    r#"{"q": "q", "num": 5, "gl": "us"}"#
)]
#[case::site_clause(
    "\"q\"",
    r#"{"search_domain_filter": ["a.com", "b.com"]}"#,
    r#"{"q": "(q) (site:a.com OR site:b.com)"}"#
)]
#[case::single_domain(
    "\"q\"",
    r#"{"search_domain_filter": ["a.com"]}"#,
    r#"{"q": "(q) (site:a.com)"}"#
)]
#[case::empty_domains("\"q\"", r#"{"search_domain_filter": []}"#, r#"{"q": "q"}"#)]
#[case::string_domains("\"q\"", r#"{"search_domain_filter": "a.com"}"#, r#"{"q": "q"}"#)]
#[case::null_domains("\"q\"", r#"{"search_domain_filter": null}"#, r#"{"q": "q"}"#)]
#[case::empty_country("\"q\"", r#"{"country": ""}"#, r#"{"q": "q", "gl": ""}"#)]
#[case::passthrough(
    "\"q\"",
    r#"{"num": 3, "q": "x", "gl": "de", "hl": "en", "country": "GB"}"#,
    r#"{"q": "q", "num": 3, "gl": "gb", "hl": "en"}"#
)]
#[case::unified_wins_over_passthrough(
    "\"q\"",
    r#"{"max_results": 5, "num": 3, "country": "US", "gl": "de"}"#,
    r#"{"q": "q", "num": 5, "gl": "us"}"#
)]
#[case::null_max_results("\"q\"", r#"{"max_results": null}"#, r#"{"q": "q", "num": null}"#)]
#[case::unified_dropped("\"q\"", r#"{"max_tokens_per_page": 5}"#, r#"{"q": "q"}"#)]
fn serper_request_maps_gl_and_domains(
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
#[case::numeric_country(r#"{"country": 5}"#)]
#[case::object_country(r#"{"country": {"a": 1}}"#)]
fn serper_request_rejects_a_non_string_country(#[case] options: &str) {
    let optional_params: Object = serde_json::from_str(options).unwrap();
    let query: SearchQuery = serde_json::from_str("\"q\"").unwrap();
    let error = config(&[])
        .transform_search_request(&query, &optional_params)
        .unwrap_err();
    assert!(matches!(
        error,
        SerperSearchError::OptionType { field: "country" }
    ));
}

#[test]
fn serper_request_rejects_a_non_string_domain() {
    let optional_params: Object = serde_json::from_str(r#"{"search_domain_filter": [1]}"#).unwrap();
    let query: SearchQuery = serde_json::from_str("\"q\"").unwrap();
    let error = config(&[])
        .transform_search_request(&query, &optional_params)
        .unwrap_err();
    assert!(matches!(
        error,
        SerperSearchError::OptionList {
            field: "search_domain_filter"
        }
    ));
}

#[rstest]
#[case::missing_organic("{}", r#"{"results": [], "object": "search"}"#)]
#[case::empty_organic(r#"{"organic": []}"#, r#"{"results": [], "object": "search"}"#)]
#[case::only_organic(
    r#"{"organic": [{"title": "t", "link": "u", "snippet": "s", "date": "2024-01-02"}], "knowledgeGraph": {"title": "k"}, "answerBox": {"answer": "a"}}"#,
    r#"{"results": [{"title": "t", "url": "u", "snippet": "s", "date": "2024-01-02", "last_updated": null}], "object": "search"}"#
)]
#[case::missing_members(
    r#"{"organic": [{}]}"#,
    r#"{"results": [{"title": "", "url": "", "snippet": "", "date": null, "last_updated": null}], "object": "search"}"#
)]
#[case::null_date(
    r#"{"organic": [{"date": null}]}"#,
    r#"{"results": [{"title": "", "url": "", "snippet": "", "date": null, "last_updated": null}], "object": "search"}"#
)]
#[case::unknown_members_dropped(
    r#"{"organic": [{"title": "t", "position": 1}]}"#,
    r#"{"results": [{"title": "t", "url": "", "snippet": "", "date": null, "last_updated": null}], "object": "search"}"#
)]
fn serper_reply_reads_organic_only(#[case] payload: &str, #[case] expected: &str) {
    let response = decode_serper_search(payload.as_bytes()).unwrap();
    let serialized = serde_json::to_string(&response).unwrap();
    let actual: IndexMap<String, Json> = serde_json::from_str(&serialized).unwrap();
    let expected_reply: IndexMap<String, Json> = serde_json::from_str(expected).unwrap();
    assert_eq!(actual, expected_reply);
}

#[test]
fn serper_reply_rejects_null_organic() {
    let error = decode_serper_search(br#"{"organic": null}"#).unwrap_err();
    assert!(matches!(error, SerperSearchError::OrganicShape));
}

#[rstest]
#[case::null_title(r#"{"organic": [{"title": null}]}"#, "title")]
#[case::null_link(r#"{"organic": [{"link": null}]}"#, "link")]
#[case::null_snippet(r#"{"organic": [{"snippet": null}]}"#, "snippet")]
fn serper_reply_rejects_a_null_member(#[case] payload: &str, #[case] expected: &str) {
    let error = decode_serper_search(payload.as_bytes()).unwrap_err();
    assert!(matches!(error, SerperSearchError::ResultField { field } if field == expected));
}

#[rstest]
#[case::string_organic(r#"{"organic": "abc"}"#)]
#[case::object_organic(r#"{"organic": {"a": 1}}"#)]
#[case::invalid_json("{")]
#[case::trailing_bytes(r#"{"organic": []}{"#)]
fn serper_reply_rejects_a_wrong_type(#[case] payload: &str) {
    let error = decode_serper_search(payload.as_bytes()).unwrap_err();
    assert!(matches!(error, SerperSearchError::ReplyDecode(_)));
}

#[rstest]
#[case::numeric_title(r#"{"organic": [{"title": 5}]}"#, "title")]
#[case::numeric_link(r#"{"organic": [{"link": 5}]}"#, "link")]
#[case::numeric_snippet(r#"{"organic": [{"snippet": 5}]}"#, "snippet")]
#[case::numeric_date(r#"{"organic": [{"date": 5}]}"#, "date")]
fn serper_reply_rejects_a_member_of_the_wrong_type(#[case] payload: &str, #[case] expected: &str) {
    let error = decode_serper_search(payload.as_bytes()).unwrap_err();
    assert!(matches!(error, SerperSearchError::ReplyField { field, .. } if field == expected));
}

#[rstest]
#[case::null_member(r#"{"organic": [null]}"#)]
#[case::array_member(r#"{"organic": [[]]}"#)]
#[case::empty_array("[]")]
#[case::null_reply("null")]
#[case::empty_reply("")]
fn serper_reply_rejects_a_value_that_is_not_an_object(#[case] payload: &str) {
    let error = decode_serper_search(payload.as_bytes()).unwrap_err();
    assert!(matches!(error, SerperSearchError::NotAnObject));
}

#[test]
fn serper_reply_rejects_a_reply_over_the_bound() {
    let oversized = vec![b' '; REPLY_MAX_BYTES + 1];
    assert!(matches!(
        decode_serper_search(&oversized),
        Err(SerperSearchError::ReplyTooLarge { .. })
    ));
}

#[test]
fn serper_reply_accepts_a_reply_at_the_bound() {
    let mut at_bound = vec![b' '; REPLY_MAX_BYTES];
    at_bound[..2].copy_from_slice(b"{}");
    let response = decode_serper_search(&at_bound).unwrap();
    assert!(response.results.is_empty());
}

#[rstest]
fn replays_the_serper_corpus(#[files("fuzz/corpus/serper_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_serper_search(&bytes));
}
