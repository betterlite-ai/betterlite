#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::CredentialError;
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, PERPLEXITY_PARAMS, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResult;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::FastcrwError;
use super::reply::{MAX_REPLY_BYTES, decode_fastcrw_reply};
use super::request::SearchBody;
use super::{FASTCRW_API_BASE, FastcrwSearch};

fn load(environment: &[(&str, &str)]) -> FastcrwSearch {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    FastcrwSearch::load(&lookup)
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

fn params(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn body_object<T>(body: &T) -> Object
where
    T: serde::Serialize,
{
    serde_json::from_str(&serde_json::to_string(body).unwrap()).unwrap()
}

fn result(title: &str, url: &str, snippet: &str) -> SearchResult {
    SearchResult {
        title: title.to_owned(),
        url: url.to_owned(),
        snippet: snippet.to_owned(),
        date: None,
        last_updated: None,
        extra: Object::new(),
    }
}

#[test]
fn fastcrw_search_settings() {
    let search = load(&[]);
    assert!(search.api_key.is_none());
    assert_eq!(search.api_base, FASTCRW_API_BASE);
}

#[rstest]
#[case::key_and_base(
    &[("CRW_API_KEY", "crw-test-key"), ("CRW_API_BASE", "https://crw.test/api/v1")],
    Some("crw-test-key"),
    "https://crw.test/api/v1"
)]
#[case::empty_key_is_unset(&[("CRW_API_KEY", "")], None, FASTCRW_API_BASE)]
#[case::empty_base_is_the_default(&[("CRW_API_BASE", "")], None, FASTCRW_API_BASE)]
#[case::whitespace_key_is_a_value(&[("CRW_API_KEY", " ")], Some(" "), FASTCRW_API_BASE)]
#[case::unrelated_variables_are_ignored(&[("OTHER", "x")], None, FASTCRW_API_BASE)]
fn settings_read_the_environment(
    #[case] environment: &[(&str, &str)],
    #[case] expected_key: Option<&str>,
    #[case] expected_base: &str,
) {
    let search = load(environment);
    assert_eq!(
        search
            .api_key
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned()),
        expected_key.map(str::to_owned)
    );
    assert_eq!(search.api_base, expected_base);
}

#[test]
fn display_name_and_method_match_the_source() {
    assert_eq!(
        <FastcrwSearch as SearchProvider>::ui_friendly_name(),
        "fastCRW"
    );
    assert_eq!(load(&[]).http_method(), HttpMethod::Post);
    assert_eq!(
        PERPLEXITY_PARAMS.join(" "),
        "max_results search_domain_filter country max_tokens_per_page"
    );
}

#[test]
fn headers_carry_bearer_and_content_type() {
    let search = load(&[("CRW_API_KEY", "crw-server")]);
    let prepared = search
        .prepare_headers(headers(&[("X-Trace", "1")]), None, None)
        .unwrap();
    assert_eq!(
        prepared,
        headers(&[
            ("Authorization", "Bearer crw-server"),
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
    assert!(matches!(error, FastcrwError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "CRW_API_KEY is not set. Set `CRW_API_KEY` environment variable."
    );
}

#[test]
fn caller_key_skips_the_trust_check() {
    let search = load(&[("CRW_API_KEY", "crw-server")]);
    let prepared = search
        .prepare_headers(
            Headers::new(),
            Some(&key("crw-caller")),
            Some("https://evil.test/search"),
        )
        .unwrap();
    assert_eq!(
        prepared.get("Authorization").map(String::as_str),
        Some("Bearer crw-caller")
    );
}

#[test]
fn empty_caller_key_falls_back_to_the_server_key() {
    let search = load(&[("CRW_API_KEY", "crw-server")]);
    let prepared = search
        .prepare_headers(Headers::new(), Some(&key("")), None)
        .unwrap();
    assert_eq!(
        prepared.get("Authorization").map(String::as_str),
        Some("Bearer crw-server")
    );
}

#[test]
fn server_key_refuses_an_untrusted_caller_base() {
    let search = load(&[("CRW_API_KEY", "crw-server")]);
    let error = search
        .prepare_headers(Headers::new(), None, Some("https://evil.test/search"))
        .unwrap_err();
    assert!(matches!(
        error,
        FastcrwError::Credential(CredentialError::UntrustedBase { .. })
    ));
    assert!(!error.to_string().contains("crw-server"));
}

#[rstest]
#[case::default_host("https://fastcrw.com/api/v1", &[], true)]
#[case::default_host_is_case_insensitive("https://FASTCRW.com/api/v1", &[], true)]
#[case::operator_override(
    "https://proxy.internal:9000/fastcrw",
    &[("CRW_API_BASE", "https://proxy.internal:9000")],
    true
)]
#[case::other_host("https://fastcrw.com.evil.test/search", &[], false)]
#[case::other_port("https://fastcrw.com:8443/search", &[], false)]
fn server_key_trust_follows_the_configured_hosts(
    #[case] api_base: &str,
    #[case] environment: &[(&str, &str)],
    #[case] trusted: bool,
) {
    let mut settings = vec![("CRW_API_KEY", "crw-server")];
    settings.extend_from_slice(environment);
    let prepared = load(&settings).prepare_headers(Headers::new(), None, Some(api_base));
    assert_eq!(prepared.is_ok(), trusted, "{prepared:?}");
}

#[rstest]
#[case::default_base(None, &[], "https://fastcrw.com/api/v1/search")]
#[case::caller_base(Some("https://proxy.test"), &[], "https://proxy.test/search")]
#[case::env_base(
    None,
    &[("CRW_API_BASE", "https://env.test/api/v1")],
    "https://env.test/api/v1/search"
)]
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
fn trait_hooks_shape_headers_url_and_options() {
    let search = load(&[("CRW_API_KEY", "crw-env")]);
    let validated = search.validate_environment(headers(&[("X-Trace", "1")]), None, None);
    assert_eq!(
        validated,
        headers(&[
            ("Authorization", "Bearer crw-env"),
            ("Content-Type", "application/json"),
            ("X-Trace", "1"),
        ])
    );
    assert_eq!(
        search
            .complete_url(Some("https://proxy.test"), &Object::new(), None)
            .unwrap(),
        "https://proxy.test/search"
    );
    let empty_caller_key = search.validate_environment(Headers::new(), Some(&key("")), None);
    assert_eq!(
        empty_caller_key.get("Authorization").map(String::as_str),
        Some("Bearer crw-env")
    );
    let keyless = load(&[]).validate_environment(headers(&[("X-Trace", "1")]), None, None);
    assert_eq!(keyless, headers(&[("X-Trace", "1")]));
    let response = ProviderResponse {
        status_code: 200,
        headers: &Headers::new(),
        text: r#"{"data":[{"title":"T","url":"https://e.test","markdown":"md"}]}"#,
    };
    let parsed = search.transform_search_response(&response).unwrap();
    assert_eq!(parsed.results, vec![result("T", "https://e.test", "md")]);
}

fn outbound<'call>(
    api_base: &'call str,
    api_key: Option<&'call SecretString>,
    params: &'call Object,
    body: &'call SearchBody,
) -> OutboundSearch<'call, SearchBody> {
    OutboundSearch {
        api_base,
        api_key,
        optional_params: params,
        body,
    }
}

fn empty_body(search: &FastcrwSearch) -> SearchBody {
    search
        .transform_search_request(&SearchQuery::Single("rust".to_owned()), &Object::new())
        .unwrap()
}

#[test]
fn trait_sign_request_returns_auth_headers() {
    let search = load(&[("CRW_API_KEY", "crw-env")]);
    let params = Object::new();
    let body = empty_body(&search);
    let request = outbound("https://fastcrw.com/api/v1/search", None, &params, &body);
    let (signed, signed_body) = search.sign_request(Headers::new(), &request).unwrap();
    assert_eq!(
        signed.get("Authorization").map(String::as_str),
        Some("Bearer crw-env")
    );
    assert!(signed_body.is_none());
}

#[test]
fn trait_sign_request_reports_a_missing_key() {
    let search = load(&[]);
    let params = Object::new();
    let body = empty_body(&search);
    let request = outbound("https://fastcrw.com/api/v1/search", None, &params, &body);
    assert!(matches!(
        search.sign_request(Headers::new(), &request),
        Err(FastcrwError::MissingApiKey)
    ));
}

#[test]
fn trait_sign_request_sends_the_caller_key_to_an_untrusted_base() {
    let search = load(&[("CRW_API_KEY", "crw-server")]);
    let caller = key("crw-caller");
    let params = Object::new();
    let body = empty_body(&search);
    let request = outbound("https://evil.test/search", Some(&caller), &params, &body);
    let (signed, signed_body) = search.sign_request(Headers::new(), &request).unwrap();
    assert_eq!(
        signed.get("Authorization").map(String::as_str),
        Some("Bearer crw-caller")
    );
    assert!(signed_body.is_none());
}

#[test]
fn trait_sign_request_rejects_an_untrusted_base() {
    let search = load(&[("CRW_API_KEY", "crw-server")]);
    let params = Object::new();
    let body = empty_body(&search);
    let request = outbound("https://evil.test/search", None, &params, &body);
    let error = search.sign_request(Headers::new(), &request).unwrap_err();
    assert!(matches!(
        error,
        FastcrwError::Credential(CredentialError::UntrustedBase { .. })
    ));
}

#[rstest]
#[case::single(SearchQuery::Single("rust".to_owned()), "rust")]
#[case::multiple(
    SearchQuery::Multiple(vec!["rust".to_owned(), "search".to_owned()]),
    "rust search"
)]
fn body_joins_the_query(#[case] query: SearchQuery, #[case] expected: &str) {
    let body = load(&[])
        .transform_search_request(&query, &Object::new())
        .unwrap();
    assert_eq!(
        body_object(&body),
        params(&format!(
            r#"{{"query":"{expected}","scrapeOptions":{{"formats":["markdown"],"onlyMainContent":true}}}}"#
        ))
    );
}

#[test]
fn body_maps_max_results_and_drops_unified_names() {
    let optional_params = params(
        r#"{"max_results":7,"search_domain_filter":["docs.rs"],"max_tokens_per_page":512,"country":"US","sources":["web","images"],"tbs":"qdr:d"}"#,
    );
    let body = load(&[])
        .transform_search_request(&SearchQuery::Single("rust".to_owned()), &optional_params)
        .unwrap();
    assert_eq!(
        body_object(&body),
        params(
            r#"{"query":"rust","limit":7,"sources":["web","images"],"tbs":"qdr:d","scrapeOptions":{"formats":["markdown"],"onlyMainContent":true}}"#
        )
    );
}

#[test]
fn body_keeps_a_caller_scrape_options_even_when_empty() {
    let empty = params(r#"{"scrapeOptions":{}}"#);
    let body = load(&[])
        .transform_search_request(&SearchQuery::Single("q".to_owned()), &empty)
        .unwrap();
    assert_eq!(
        body_object(&body),
        params(r#"{"query":"q","scrapeOptions":{}}"#)
    );
    let custom = params(r#"{"scrapeOptions":{"formats":["html"]}}"#);
    let custom_body = load(&[])
        .transform_search_request(&SearchQuery::Single("q".to_owned()), &custom)
        .unwrap();
    assert_eq!(
        body_object(&custom_body),
        params(r#"{"query":"q","scrapeOptions":{"formats":["html"]}}"#)
    );
}

#[test]
fn body_never_overwrites_a_mapped_name() {
    let optional_params = params(r#"{"max_results":3,"limit":9,"query":"ignored"}"#);
    let body = load(&[])
        .transform_search_request(&SearchQuery::Single("rust".to_owned()), &optional_params)
        .unwrap();
    assert_eq!(
        body_object(&body),
        params(
            r#"{"query":"rust","limit":3,"scrapeOptions":{"formats":["markdown"],"onlyMainContent":true}}"#
        )
    );
    let unmapped = params(r#"{"limit":9}"#);
    let unmapped_body = load(&[])
        .transform_search_request(&SearchQuery::Single("rust".to_owned()), &unmapped)
        .unwrap();
    assert_eq!(
        body_object(&unmapped_body),
        params(
            r#"{"query":"rust","limit":9,"scrapeOptions":{"formats":["markdown"],"onlyMainContent":true}}"#
        )
    );
}

#[test]
fn reply_projects_flat_data_with_markdown_then_description() {
    let parsed = decode_fastcrw_reply(
        br#"{"success":true,"data":[
            {"title":"A","url":"https://a.test","markdown":"body","description":"desc"},
            {"title":"B","url":"https://b.test","description":"fallback"},
            {"title":"C","url":"https://c.test","markdown":""}
        ]}"#,
    )
    .unwrap();
    assert_eq!(parsed.object.0, "search");
    assert_eq!(
        parsed.results,
        vec![
            result("A", "https://a.test", "body"),
            result("B", "https://b.test", "fallback"),
            result("C", "https://c.test", ""),
        ]
    );
}

#[test]
fn reply_defaults_missing_fields_to_empty_strings() {
    let parsed = decode_fastcrw_reply(br#"{"data":[{}]}"#).unwrap();
    assert_eq!(parsed.results, vec![result("", "", "")]);
    assert_eq!(
        serde_json::to_string(&parsed).unwrap(),
        r#"{"results":[{"title":"","url":"","snippet":"","date":null,"last_updated":null}],"object":"search"}"#
    );
}

#[rstest]
#[case::object_data(br#"{"data":{"web":[]}}"#)]
#[case::string_data(br#"{"data":"text"}"#)]
#[case::numeric_data(br#"{"data":5}"#)]
#[case::null_data(br#"{"data":null}"#)]
#[case::missing_data(b"{}")]
#[case::success_only(br#"{"success":true}"#)]
fn reply_ignores_data_that_is_not_a_list(#[case] bytes: &[u8]) {
    let parsed = decode_fastcrw_reply(bytes).unwrap();
    assert!(parsed.results.is_empty());
    assert_eq!(parsed.object.0, "search");
}

fn rejection_kind(error: &FastcrwError) -> &'static str {
    match error {
        FastcrwError::Reply { .. } => "reply",
        FastcrwError::Result { .. } => "result",
        FastcrwError::NotText { .. } => "text",
        FastcrwError::TrailingBytes(_) => "trailing",
        FastcrwError::Credential(_) => "credential",
        FastcrwError::MissingApiKey => "missing key",
        FastcrwError::ReplyTooLarge { .. } => "too large",
    }
}

#[rstest]
#[case::array_document(b"[]", "reply")]
#[case::nested_array_document(br#"[[{"title":"x"}]]"#, "reply")]
#[case::string_document(br#""text""#, "reply")]
#[case::item_not_an_object(br#"{"data":["x"]}"#, "result")]
#[case::null_item(br#"{"data":[null]}"#, "result")]
#[case::null_title(br#"{"data":[{"title":null}]}"#, "text")]
#[case::numeric_title(br#"{"data":[{"title":7}]}"#, "text")]
#[case::numeric_url(br#"{"data":[{"url":7}]}"#, "text")]
#[case::null_description_without_markdown(br#"{"data":[{"description":null}]}"#, "text")]
fn reply_rejects_a_shape_the_source_would_fail(#[case] bytes: &[u8], #[case] kind: &str) {
    let error = decode_fastcrw_reply(bytes).unwrap_err();
    assert_eq!(rejection_kind(&error), kind);
}

#[test]
fn reply_accepts_a_null_description_when_markdown_covers_it() {
    let parsed =
        decode_fastcrw_reply(br#"{"data":[{"markdown":"md","description":null}]}"#).unwrap();
    assert_eq!(parsed.results, vec![result("", "", "md")]);
}

#[test]
fn reply_accepts_a_null_markdown() {
    let parsed =
        decode_fastcrw_reply(br#"{"data":[{"markdown":null,"description":"d"}]}"#).unwrap();
    assert_eq!(parsed.results, vec![result("", "", "d")]);
}

#[test]
fn reply_ignores_item_dates() {
    let parsed = decode_fastcrw_reply(
        br#"{"data":[{"title":"A","url":"https://a.test","date":"2024-01-02","last_updated":"2024-02-03"}]}"#,
    )
    .unwrap();
    assert_eq!(parsed.results, vec![result("A", "https://a.test", "")]);
    assert!(parsed.results[0].date.is_none());
    assert!(parsed.results[0].last_updated.is_none());
}

#[test]
fn reply_rejects_invalid_json() {
    assert!(matches!(
        decode_fastcrw_reply(b"not json"),
        Err(FastcrwError::Reply { .. })
    ));
}

#[test]
fn reply_rejects_trailing_bytes() {
    assert!(matches!(
        decode_fastcrw_reply(br#"{"data":[]}x"#),
        Err(FastcrwError::TrailingBytes(_))
    ));
}

#[test]
fn reply_is_bounded() {
    let mut bytes = vec![b' '; MAX_REPLY_BYTES + 1];
    assert!(matches!(
        decode_fastcrw_reply(&bytes),
        Err(FastcrwError::ReplyTooLarge {
            limit: MAX_REPLY_BYTES
        })
    ));
    bytes.truncate(MAX_REPLY_BYTES);
    assert!(matches!(
        decode_fastcrw_reply(&bytes),
        Err(FastcrwError::Reply { .. })
    ));
}

#[rstest]
fn replays_the_fastcrw_corpus(#[files("fuzz/corpus/fastcrw_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(parsed) = decode_fastcrw_reply(&bytes) {
        assert_eq!(parsed.object.0, "search");
        assert!(parsed.results.iter().all(|result| result.date.is_none()));
        assert!(
            parsed
                .results
                .iter()
                .all(|result| result.last_updated.is_none())
        );
    }
}
