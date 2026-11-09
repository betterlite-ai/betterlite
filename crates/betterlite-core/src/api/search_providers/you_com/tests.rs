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

use super::error::YouComError;
use super::reply::{MAX_REPLY_BYTES, decode_you_com_reply};
use super::{YOU_COM_FREE_API_BASE, YouComSearch};

fn load(environment: &[(&str, &str)]) -> YouComSearch {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    YouComSearch::load(&lookup)
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
fn you_com_search_settings_default_to_unset() {
    let search = load(&[]);
    assert!(search.api_key.is_none());
    assert!(search.api_base.is_none());
}

#[rstest]
#[case::key_and_base(
    &[("YOUCOM_API_KEY", "sk-test"), ("YOUCOM_API_BASE", "https://proxy.test")],
    Some("sk-test"),
    Some("https://proxy.test")
)]
#[case::empty_key_is_unset(&[("YOUCOM_API_KEY", "")], None, None)]
#[case::empty_base_is_unset(&[("YOUCOM_API_BASE", "")], None, None)]
#[case::unrelated_variables_are_ignored(&[("OTHER", "x")], None, None)]
fn you_com_search_settings_read_the_environment(
    #[case] environment: &[(&str, &str)],
    #[case] expected_key: Option<&str>,
    #[case] expected_base: Option<&str>,
) {
    let search = load(environment);
    assert_eq!(
        search.api_key.as_ref().map(ExposeSecret::expose_secret),
        expected_key
    );
    assert_eq!(search.api_base.as_deref(), expected_base);
}

#[test]
fn display_name_and_method_match_the_source() {
    assert_eq!(
        <YouComSearch as SearchProvider>::ui_friendly_name(),
        "You.com"
    );
    assert_eq!(load(&[]).http_method(), HttpMethod::Post);
}

#[test]
fn keyless_settings_resolve_to_no_key() {
    assert!(load(&[]).resolve_api_key(None, None).unwrap().is_none());
}

#[test]
fn environment_key_resolves_when_the_base_is_absent() {
    let resolved = load(&[("YOUCOM_API_KEY", "sk-env")])
        .resolve_api_key(None, None)
        .unwrap();
    assert_eq!(
        resolved.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-env")
    );
}

#[test]
fn caller_key_skips_the_trust_check() {
    let resolved = load(&[("YOUCOM_API_KEY", "sk-env")])
        .resolve_api_key(Some(&key("sk-caller")), Some("https://evil.test"))
        .unwrap();
    assert_eq!(
        resolved.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-caller")
    );
}

#[test]
fn environment_key_refuses_an_untrusted_caller_base() {
    let error = load(&[("YOUCOM_API_KEY", "sk-env")])
        .resolve_api_key(None, Some("https://evil.test"))
        .unwrap_err();
    assert!(matches!(
        error,
        YouComError::Credential(CredentialError::UntrustedBase { .. })
    ));
    assert!(!error.to_string().contains("sk-env"));
}

#[test]
fn environment_key_refuses_the_keyless_free_host() {
    let error = load(&[("YOUCOM_API_KEY", "sk-env")])
        .prepare_headers(Headers::new(), None, Some(YOU_COM_FREE_API_BASE))
        .unwrap_err();
    assert!(matches!(
        error,
        YouComError::Credential(CredentialError::UntrustedBase { .. })
    ));
}

#[test]
fn untrusted_base_without_a_server_key_is_keyless() {
    assert!(
        load(&[])
            .resolve_api_key(None, Some("https://evil.test"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn keyless_headers_pin_content_type_and_identity_encoding() {
    let prepared = load(&[])
        .prepare_headers(headers(&[("X-Trace", "1")]), None, None)
        .unwrap();
    assert_eq!(
        prepared,
        headers(&[
            ("Accept-Encoding", "identity"),
            ("Content-Type", "application/json"),
            ("X-Trace", "1"),
        ])
    );
}

#[rstest]
#[case::exact_key("Accept-Encoding", "gzip", "gzip", 2)]
#[case::other_case("accept-encoding", "gzip", "identity", 3)]
fn identity_encoding_is_set_only_when_the_exact_key_is_absent(
    #[case] name: &str,
    #[case] value: &str,
    #[case] expected_encoding: &str,
    #[case] expected_len: usize,
) {
    let prepared = load(&[])
        .prepare_headers(headers(&[(name, value)]), None, None)
        .unwrap();
    assert_eq!(prepared.get(name).map(String::as_str), Some(value));
    assert_eq!(
        prepared.get("Accept-Encoding").map(String::as_str),
        Some(expected_encoding)
    );
    assert_eq!(prepared.len(), expected_len);
}

#[test]
fn headers_carry_the_resolved_key() {
    let prepared = load(&[("YOUCOM_API_KEY", "sk-env")])
        .prepare_headers(Headers::new(), None, None)
        .unwrap();
    assert_eq!(
        prepared.get("X-API-Key").map(String::as_str),
        Some("sk-env")
    );
    let caller = load(&[])
        .prepare_headers(Headers::new(), Some(&key("sk-caller")), None)
        .unwrap();
    assert_eq!(
        caller.get("X-API-Key").map(String::as_str),
        Some("sk-caller")
    );
}

#[rstest]
#[case::keyless_free_url(None, &[], "https://api.you.com/v1/agents/search")]
#[case::environment_key_selects_the_keyed_default(
    None,
    &[("YOUCOM_API_KEY", "sk-env")],
    "https://ydc-index.io/v1/search"
)]
#[case::environment_base_wins_over_the_keyed_default(
    None,
    &[("YOUCOM_API_BASE", "https://env.test")],
    "https://env.test/v1/search"
)]
#[case::caller_base_wins_over_the_environment_base(
    Some("https://caller.test"),
    &[("YOUCOM_API_BASE", "https://env.test")],
    "https://caller.test/v1/search"
)]
#[case::trailing_slashes_are_trimmed(
    Some("https://caller.test///"),
    &[],
    "https://caller.test/v1/search"
)]
#[case::search_suffix_is_kept(
    Some("https://caller.test/v1/search"),
    &[],
    "https://caller.test/v1/search"
)]
#[case::agents_suffix_is_kept(
    Some("https://caller.test/v1/agents/search"),
    &[],
    "https://caller.test/v1/agents/search"
)]
#[case::other_path_gets_the_suffix(
    Some("https://caller.test/v1"),
    &[],
    "https://caller.test/v1/v1/search"
)]
#[case::empty_caller_base_is_used_as_given(Some(""), &[], "/v1/search")]
fn complete_url_picks_the_keyed_or_free_base(
    #[case] api_base: Option<&str>,
    #[case] environment: &[(&str, &str)],
    #[case] expected: &str,
) {
    assert_eq!(
        load(environment)
            .complete_url(api_base, &Object::new(), None)
            .unwrap(),
        expected
    );
}

#[test]
fn caller_key_does_not_reach_the_url_choice() {
    // B3270's complete_url takes no api_key, so a caller key shapes headers only (docs/deviations/B8021.yaml)
    let search = load(&[]);
    let prepared = search
        .prepare_headers(Headers::new(), Some(&key("sk-caller")), None)
        .unwrap();
    assert_eq!(
        prepared.get("X-API-Key").map(String::as_str),
        Some("sk-caller")
    );
    assert_eq!(
        search.complete_url(None, &Object::new(), None).unwrap(),
        YOU_COM_FREE_API_BASE
    );
}

#[test]
fn sign_request_refuses_the_server_key_for_the_free_host() {
    let search = load(&[("YOUCOM_API_KEY", "sk-env")]);
    let optional_params = Object::new();
    let body = Object::new();
    let request = OutboundSearch {
        api_base: YOU_COM_FREE_API_BASE,
        api_key: None,
        optional_params: &optional_params,
        body: &body,
    };
    let error = search.sign_request(Headers::new(), &request).unwrap_err();
    assert!(matches!(
        error,
        YouComError::Credential(CredentialError::UntrustedBase { .. })
    ));
}

#[test]
fn sign_request_refuses_the_server_key_for_an_untrusted_host() {
    let search = load(&[("YOUCOM_API_KEY", "sk-env")]);
    let optional_params = Object::new();
    let body = Object::new();
    let request = OutboundSearch {
        api_base: "https://evil.test/v1/search",
        api_key: None,
        optional_params: &optional_params,
        body: &body,
    };
    let error = search.sign_request(Headers::new(), &request).unwrap_err();
    assert!(matches!(
        error,
        YouComError::Credential(CredentialError::UntrustedBase { .. })
    ));
}

#[test]
fn sign_request_adds_the_server_key_for_a_trusted_host() {
    let search = load(&[("YOUCOM_API_KEY", "sk-env")]);
    let optional_params = Object::new();
    let body = Object::new();
    let request = OutboundSearch {
        api_base: "https://ydc-index.io/v1/search",
        api_key: None,
        optional_params: &optional_params,
        body: &body,
    };
    let (headers, signed) = search.sign_request(Headers::new(), &request).unwrap();
    assert_eq!(headers.get("X-API-Key").map(String::as_str), Some("sk-env"));
    assert_eq!(signed, None);
}

#[test]
fn sign_request_accepts_a_caller_key_on_an_untrusted_host() {
    let search = load(&[("YOUCOM_API_KEY", "sk-env")]);
    let optional_params = Object::new();
    let body = Object::new();
    let caller_key = key("sk-caller");
    let request = OutboundSearch {
        api_base: "https://evil.test/v1/search",
        api_key: Some(&caller_key),
        optional_params: &optional_params,
        body: &body,
    };
    let (headers, _signed) = search.sign_request(Headers::new(), &request).unwrap();
    assert_eq!(
        headers.get("X-API-Key").map(String::as_str),
        Some("sk-caller")
    );
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
            "safesearch": "strict",
            "country": "FR",
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
                "count": 7,
                "include_domains": ["docs.rs"],
                "country": "fr",
                "safesearch": "strict"
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
    let optional_params = object(r#"{"country": []}"#);
    let error = load(&[])
        .transform_search_request(&SearchQuery::Single("rust".to_owned()), &optional_params)
        .unwrap_err();
    assert!(matches!(error, YouComError::CountryNotString));
}

#[test]
fn reply_reads_web_then_news() {
    let response = decode_you_com_reply(
        br#"{"results":{"web":[{"title":"Web","url":"https://web.test","snippets":["first","second"],"description":"ignored","page_age":"2024-01-02"}],"news":[{"title":"News","url":"https://news.test","snippets":[],"description":"fallback"}]}}"#,
    )
    .unwrap();
    assert_eq!(response.object.0, "search");
    assert_eq!(
        response.results,
        vec![
            SearchResult {
                title: "Web".to_owned(),
                url: "https://web.test".to_owned(),
                snippet: "first".to_owned(),
                date: Some("2024-01-02".to_owned()),
                last_updated: None,
                extra: Object::new(),
            },
            SearchResult {
                title: "News".to_owned(),
                url: "https://news.test".to_owned(),
                snippet: "fallback".to_owned(),
                date: None,
                last_updated: None,
                extra: Object::new(),
            },
        ]
    );
}

#[rstest]
#[case::no_results(b"{}", 0)]
#[case::null_results(br#"{"results":null}"#, 0)]
#[case::empty_results_list(br#"{"results":[]}"#, 0)]
#[case::null_web(br#"{"results":{"web":null}}"#, 0)]
#[case::null_news(br#"{"results":{"news":null}}"#, 0)]
#[case::news_only(br#"{"results":{"news":[{"title":"N"}]}}"#, 1)]
fn reply_treats_absent_empty_and_null_lists_as_empty(
    #[case] bytes: &[u8],
    #[case] expected: usize,
) {
    assert_eq!(decode_you_com_reply(bytes).unwrap().results.len(), expected);
}

#[rstest]
#[case::missing_description(br#"{"results":{"web":[{"title":"T","snippets":[]}]}}"#, "")]
#[case::null_snippets(
    br#"{"results":{"web":[{"title":"T","snippets":null,"description":"d"}]}}"#,
    "d"
)]
#[case::missing_snippets(br#"{"results":{"web":[{"title":"T","description":"d"}]}}"#, "d")]
fn reply_falls_back_to_description(#[case] bytes: &[u8], #[case] expected: &str) {
    let response = decode_you_com_reply(bytes).unwrap();
    assert_eq!(response.results[0].snippet, expected);
}

#[rstest]
#[case::null_title(br#"{"results":{"web":[{"title":null}]}}"#)]
#[case::null_url(br#"{"results":{"web":[{"url":null}]}}"#)]
#[case::numeric_page_age(br#"{"results":{"web":[{"title":"T","page_age":7}]}}"#)]
#[case::null_snippet(br#"{"results":{"web":[{"title":"T","snippets":[null]}]}}"#)]
#[case::snippets_not_a_list(br#"{"results":{"web":[{"title":"T","snippets":"x"}]}}"#)]
#[case::nonempty_results_list(br#"{"results":[[{"title":"T"}]]}"#)]
#[case::not_an_object(b"[]")]
#[case::item_is_a_list(br#"{"results":{"web":[["T"]]}}"#)]
#[case::web_not_a_list(br#"{"results":{"web":{}}}"#)]
fn reply_rejects_a_shape_the_source_would_fail(#[case] bytes: &[u8]) {
    assert!(matches!(
        decode_you_com_reply(bytes),
        Err(YouComError::Reply { .. })
    ));
}

#[test]
fn reply_ignores_a_null_description_when_snippets_exist() {
    let response = decode_you_com_reply(
        br#"{"results":{"web":[{"title":"T","url":"u","snippets":["a"],"description":null}]}}"#,
    )
    .unwrap();
    assert_eq!(response.results[0].snippet, "a");
}

#[test]
fn reply_rejects_a_null_description_without_snippets() {
    assert!(matches!(
        decode_you_com_reply(
            br#"{"results":{"web":[{"title":"T","snippets":[],"description":null}]}}"#
        ),
        Err(YouComError::DescriptionNotString)
    ));
}

#[test]
fn reply_rejects_invalid_json() {
    assert!(matches!(
        decode_you_com_reply(b"not json"),
        Err(YouComError::Reply { .. })
    ));
}

#[test]
fn reply_rejects_trailing_bytes() {
    assert!(matches!(
        decode_you_com_reply(br#"{"results":{}}x"#),
        Err(YouComError::TrailingBytes(_))
    ));
}

#[test]
fn reply_is_bounded() {
    let mut bytes = vec![b' '; MAX_REPLY_BYTES + 1];
    assert!(matches!(
        decode_you_com_reply(&bytes),
        Err(YouComError::ReplyTooLarge {
            limit: MAX_REPLY_BYTES
        })
    ));
    bytes.truncate(MAX_REPLY_BYTES);
    assert!(matches!(
        decode_you_com_reply(&bytes),
        Err(YouComError::Reply { .. })
    ));
}

#[test]
fn trait_hooks_shape_headers_url_and_reply() {
    let search = load(&[]);
    let validated = search.validate_environment(headers(&[("X-Trace", "1")]), None, None);
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    let response = ProviderResponse {
        status_code: 200,
        headers: &Headers::new(),
        text: r#"{"results":{"web":[{"title":"T"}]}}"#,
    };
    let parsed = search.transform_search_response(&response).unwrap();
    assert_eq!(parsed.results.len(), 1);
}

#[test]
fn trait_validate_environment_prefers_the_caller_key() {
    let search = load(&[("YOUCOM_API_KEY", "sk-env")]);
    let validated = search.validate_environment(Headers::new(), Some(&key("sk-caller")), None);
    assert_eq!(
        validated.get("X-API-Key").map(String::as_str),
        Some("sk-caller")
    );
}

#[test]
fn trait_validate_environment_uses_a_caller_key_without_an_environment_key() {
    let search = load(&[]);
    let validated = search.validate_environment(Headers::new(), Some(&key("sk-caller")), None);
    assert_eq!(
        validated.get("X-API-Key").map(String::as_str),
        Some("sk-caller")
    );
}

#[test]
fn server_key_trust_follows_the_configured_base() {
    let search = load(&[
        ("YOUCOM_API_KEY", "sk-env"),
        ("YOUCOM_API_BASE", "https://proxy.test"),
    ]);
    let prepared = search
        .prepare_headers(Headers::new(), None, Some("https://proxy.test/v1/search"))
        .unwrap();
    assert_eq!(
        prepared.get("X-API-Key").map(String::as_str),
        Some("sk-env")
    );
}

#[test]
fn server_key_is_trusted_for_the_keyed_default_host() {
    let search = load(&[("YOUCOM_API_KEY", "sk-env")]);
    let prepared = search
        .prepare_headers(Headers::new(), None, Some("https://ydc-index.io/v1/search"))
        .unwrap();
    assert_eq!(
        prepared.get("X-API-Key").map(String::as_str),
        Some("sk-env")
    );
}

#[rstest]
fn replays_the_you_com_corpus(#[files("fuzz/corpus/you_com_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(response) = decode_you_com_reply(&bytes) {
        assert_eq!(response.object.0, "search");
        assert!(
            response
                .results
                .iter()
                .all(|result| result.last_updated.is_none())
        );
    }
}
