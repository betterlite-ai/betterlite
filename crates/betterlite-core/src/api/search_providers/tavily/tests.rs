#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::CredentialError;
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResult;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::error::TavilyError;
use super::reply::{MAX_REPLY_BYTES, decode_tavily_reply};
use super::{TAVILY_API_BASE, TavilySearch};

fn load(environment: &[(&str, &str)]) -> TavilySearch {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    TavilySearch::load(&lookup)
}
fn key(text: &str) -> SecretString {
    SecretString::from(text)
}
fn headers(entries: &[(&str, &str)]) -> Headers {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}
fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

#[test]
fn tavily_search_settings_default_to_the_published_base() {
    let search = load(&[]);
    assert!(search.api_key.is_none());
    assert_eq!(search.api_base, TAVILY_API_BASE);
}

#[rstest]
#[case::key_and_base(
    &[("TAVILY_API_KEY", "sk-test"), ("TAVILY_API_BASE", "https://proxy.test")],
    Some("sk-test"),
    "https://proxy.test"
)]
#[case::empty_key_is_unset(&[("TAVILY_API_KEY", "")], None, TAVILY_API_BASE)]
#[case::empty_base_is_the_default(&[("TAVILY_API_BASE", "")], None, TAVILY_API_BASE)]
#[case::whitespace_key_is_a_value(&[("TAVILY_API_KEY", " ")], Some(" "), TAVILY_API_BASE)]
#[case::unrelated_variables_are_ignored(&[("OTHER", "x")], None, TAVILY_API_BASE)]
fn tavily_search_settings_read_the_environment(
    #[case] environment: &[(&str, &str)],
    #[case] expected_key: Option<&str>,
    #[case] expected_base: &str,
) {
    let search = load(environment);
    assert_eq!(
        search.api_key.as_ref().map(ExposeSecret::expose_secret),
        expected_key
    );
    assert_eq!(search.api_base, expected_base);
}

#[test]
fn display_name_and_method_match_the_source() {
    assert_eq!(
        <TavilySearch as SearchProvider>::ui_friendly_name(),
        "Tavily"
    );
    assert_eq!(load(&[]).http_method(), HttpMethod::Post);
}

#[test]
fn headers_carry_bearer_and_content_type() {
    let search = load(&[("TAVILY_API_KEY", "sk-server")]);
    let prepared = search
        .prepare_headers(headers(&[("X-Trace", "1")]), None, None)
        .unwrap();
    assert_eq!(
        prepared,
        headers(&[
            ("Authorization", "Bearer sk-server"),
            ("Content-Type", "application/json"),
            ("X-Trace", "1"),
        ])
    );
}

#[test]
fn missing_key_names_the_variable() {
    let error = load(&[])
        .prepare_headers(Headers::new(), None, None)
        .unwrap_err();
    assert!(matches!(error, TavilyError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "TAVILY_API_KEY is not set. Set `TAVILY_API_KEY` environment variable."
    );
}

#[test]
fn caller_key_skips_the_trust_check() {
    let search = load(&[("TAVILY_API_KEY", "sk-server")]);
    let prepared = search
        .prepare_headers(
            Headers::new(),
            Some(&key("sk-caller")),
            Some("https://evil.test/search"),
        )
        .unwrap();
    assert_eq!(
        prepared.get("Authorization").map(String::as_str),
        Some("Bearer sk-caller")
    );
}

#[test]
fn empty_caller_key_falls_back_to_the_server_key() {
    let search = load(&[("TAVILY_API_KEY", "sk-server")]);
    let prepared = search
        .prepare_headers(Headers::new(), Some(&key("")), None)
        .unwrap();
    assert_eq!(
        prepared.get("Authorization").map(String::as_str),
        Some("Bearer sk-server")
    );
}

#[test]
fn server_key_refuses_an_untrusted_caller_base() {
    let search = load(&[("TAVILY_API_KEY", "sk-server")]);
    let error = search
        .prepare_headers(Headers::new(), None, Some("https://evil.test/search"))
        .unwrap_err();
    assert!(matches!(
        error,
        TavilyError::Credential(CredentialError::UntrustedBase { .. })
    ));
    assert!(!error.to_string().contains("sk-server"));
}

#[rstest]
#[case::default_host("https://api.tavily.com/v2", &[], true)]
#[case::operator_override(
    "https://proxy.internal:9000/tavily",
    &[("TAVILY_API_BASE", "https://proxy.internal:9000")],
    true
)]
#[case::other_host("https://api.tavily.com.evil.test/search", &[], false)]
#[case::other_port("https://api.tavily.com:8443/search", &[], false)]
fn server_key_trust_follows_the_configured_hosts(
    #[case] api_base: &str,
    #[case] environment: &[(&str, &str)],
    #[case] trusted: bool,
) {
    let mut settings = vec![("TAVILY_API_KEY", "sk-server")];
    settings.extend_from_slice(environment);
    let prepared = load(&settings).prepare_headers(Headers::new(), None, Some(api_base));
    assert_eq!(prepared.is_ok(), trusted, "{prepared:?}");
}

#[rstest]
#[case::default_base(None, &[], "https://api.tavily.com/search")]
#[case::caller_base(Some("https://proxy.test"), &[], "https://proxy.test/search")]
#[case::env_base(None, &[("TAVILY_API_BASE", "https://env.test")], "https://env.test/search")]
#[case::existing_suffix(
    Some("https://proxy.test/search"),
    &[],
    "https://proxy.test/search"
)]
#[case::double_slash(Some("https://proxy.test/"), &[], "https://proxy.test//search")]
#[case::lookalike_suffix(
    Some("https://proxy.test/mysearch"),
    &[],
    "https://proxy.test/mysearch/search"
)]
#[case::empty_caller_base_is_used_as_given(Some(""), &[], "/search")]
fn endpoint_appends_search_without_trimming(
    #[case] api_base: Option<&str>,
    #[case] environment: &[(&str, &str)],
    #[case] expected: &str,
) {
    assert_eq!(load(environment).endpoint(api_base), expected);
}

#[test]
fn trait_complete_url_appends_search() {
    let search = load(&[]);
    assert_eq!(
        search
            .complete_url(Some("https://proxy.test"), &Object::new(), None)
            .unwrap(),
        "https://proxy.test/search"
    );
}

#[test]
fn sign_request_refuses_a_server_key_for_an_untrusted_base() {
    let search = load(&[("TAVILY_API_KEY", "sk-server")]);
    let optional_params = Object::new();
    let body = Object::new();
    let request = OutboundSearch {
        api_base: "https://evil.test/search",
        api_key: None,
        optional_params: &optional_params,
        body: &body,
    };
    let error = search.sign_request(Headers::new(), &request).unwrap_err();
    assert!(matches!(
        error,
        TavilyError::Credential(CredentialError::UntrustedBase { .. })
    ));
}

#[test]
fn sign_request_fails_without_a_key() {
    let search = load(&[]);
    let optional_params = Object::new();
    let body = Object::new();
    let request = OutboundSearch {
        api_base: "https://api.tavily.com/search",
        api_key: None,
        optional_params: &optional_params,
        body: &body,
    };
    let error = search.sign_request(Headers::new(), &request).unwrap_err();
    assert!(matches!(error, TavilyError::MissingApiKey));
}

#[test]
fn sign_request_signs_a_server_key_for_the_trusted_host() {
    let search = load(&[("TAVILY_API_KEY", "sk-server")]);
    let optional_params = Object::new();
    let body = Object::new();
    let request = OutboundSearch {
        api_base: "https://api.tavily.com/search",
        api_key: None,
        optional_params: &optional_params,
        body: &body,
    };
    let (headers, signed) = search.sign_request(Headers::new(), &request).unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-server")
    );
    assert_eq!(signed, None);
}

#[rstest]
#[case::single(SearchQuery::Single("rust".to_owned()), r#"{"query":"rust"}"#)]
#[case::multiple(
    SearchQuery::Multiple(vec!["rust".to_owned(), "search".to_owned()]),
    r#"{"query":"rust search"}"#
)]
fn body_joins_the_query(#[case] query: SearchQuery, #[case] expected: &str) {
    let body = load(&[])
        .transform_search_request(&query, &Object::new())
        .unwrap();
    assert_eq!(body, object(expected));
}

#[test]
fn body_maps_unified_parameters_and_copies_the_rest() {
    let optional_params = object(
        r#"{
            "max_results": 7,
            "search_domain_filter": ["docs.rs"],
            "include_domains": ["ignored.test"],
            "exclude_domains": ["spam.test"],
            "topic": "news",
            "country": "US",
            "max_tokens_per_page": 512
        }"#,
    );
    let body = load(&[])
        .transform_search_request(&SearchQuery::Single("rust".to_owned()), &optional_params)
        .unwrap();
    assert_eq!(
        body,
        object(
            r#"{
                "query": "rust",
                "max_results": 7,
                "include_domains": ["docs.rs"],
                "country": "us",
                "exclude_domains": ["spam.test"],
                "topic": "news"
            }"#
        )
    );
}

#[test]
fn body_keeps_a_caller_include_domains_without_the_filter() {
    let optional_params = object(r#"{"include_domains": ["kept.test"]}"#);
    let body = load(&[])
        .transform_search_request(&SearchQuery::Single("rust".to_owned()), &optional_params)
        .unwrap();
    assert_eq!(
        body,
        object(r#"{"query": "rust", "include_domains": ["kept.test"]}"#)
    );
}

#[test]
fn body_rejects_a_non_string_country() {
    let optional_params = object(r#"{"country": 1}"#);
    let error = load(&[])
        .transform_search_request(&SearchQuery::Single("rust".to_owned()), &optional_params)
        .unwrap_err();
    assert!(matches!(error, TavilyError::CountryNotString));
}

#[test]
fn reply_projects_title_url_and_content() {
    let response = decode_tavily_reply(
        br#"{"results":[{"title":"Rust","url":"https://rust-lang.org","content":"A language","score":0.9}]}"#,
    )
    .unwrap();
    assert_eq!(response.object.0, "search");
    assert_eq!(
        response.results,
        vec![SearchResult {
            title: "Rust".to_owned(),
            url: "https://rust-lang.org".to_owned(),
            snippet: "A language".to_owned(),
            date: None,
            last_updated: None,
            extra: Object::new(),
        }]
    );
}

#[test]
fn reply_without_results_is_empty() {
    assert!(decode_tavily_reply(b"{}").unwrap().results.is_empty());
}

#[rstest]
#[case::null_title(br#"{"results":[{"title":null}]}"#)]
#[case::null_url(br#"{"results":[{"url":null}]}"#)]
#[case::null_content(br#"{"results":[{"content":null}]}"#)]
#[case::numeric_title(br#"{"results":[{"title":7}]}"#)]
#[case::null_results(br#"{"results":null}"#)]
#[case::result_not_an_object(br#"{"results":["x"]}"#)]
#[case::result_is_a_list(br#"{"results":[["t"]]}"#)]
#[case::not_an_object(b"[]")]
fn reply_rejects_a_shape_the_source_would_fail(#[case] bytes: &[u8]) {
    assert!(matches!(
        decode_tavily_reply(bytes),
        Err(TavilyError::Reply { .. })
    ));
}

#[test]
fn reply_rejects_invalid_json() {
    assert!(matches!(
        decode_tavily_reply(b"not json"),
        Err(TavilyError::Reply { .. })
    ));
}

#[test]
fn reply_rejects_trailing_bytes() {
    assert!(matches!(
        decode_tavily_reply(br#"{"results":[]}x"#),
        Err(TavilyError::TrailingBytes(_))
    ));
}

#[test]
fn reply_is_bounded() {
    let mut bytes = vec![b' '; MAX_REPLY_BYTES + 1];
    assert!(matches!(
        decode_tavily_reply(&bytes),
        Err(TavilyError::ReplyTooLarge {
            limit: MAX_REPLY_BYTES
        })
    ));
    bytes.truncate(MAX_REPLY_BYTES);
    assert!(matches!(
        decode_tavily_reply(&bytes),
        Err(TavilyError::Reply { .. })
    ));
}

#[test]
fn trait_hooks_shape_headers_url_and_reply() {
    let search = load(&[("TAVILY_API_KEY", "sk-env")]);
    let validated = search.validate_environment(headers(&[("X-Trace", "1")]), None, None);
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer sk-env")
    );
    let response = ProviderResponse {
        status_code: 200,
        headers: &Headers::new(),
        text: r#"{"results":[{"title":"T"}]}"#,
    };
    let parsed = search.transform_search_response(&response).unwrap();
    assert_eq!(parsed.results.len(), 1);
}

#[test]
fn trait_validate_environment_prefers_the_caller_key() {
    let search = load(&[("TAVILY_API_KEY", "sk-env")]);
    let validated = search.validate_environment(Headers::new(), Some(&key("sk-caller")), None);
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer sk-caller")
    );
}

#[test]
fn trait_validate_environment_uses_a_caller_key_without_an_environment_key() {
    let search = load(&[]);
    let validated = search.validate_environment(Headers::new(), Some(&key("sk-caller")), None);
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer sk-caller")
    );
}

#[rstest]
fn replays_the_tavily_corpus(#[files("fuzz/corpus/tavily_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(response) = decode_tavily_reply(&bytes) {
        assert_eq!(response.object.0, "search");
        assert!(response.results.iter().all(|result| result.date.is_none()));
        assert!(
            response
                .results
                .iter()
                .all(|result| result.last_updated.is_none())
        );
    }
}
