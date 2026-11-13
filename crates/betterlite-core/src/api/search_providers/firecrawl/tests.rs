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
use secrecy::{ExposeSecret as _, SecretString};

use super::error::FirecrawlError;
use super::reply::{MAX_REPLY_BYTES, decode_firecrawl_reply};
use super::request::SearchBody;
use super::{FIRECRAWL_API_BASE, FirecrawlSearch};

fn load(environment: &[(&str, &str)]) -> FirecrawlSearch {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    FirecrawlSearch::load(&lookup)
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

fn result(title: &str, url: &str, snippet: &str, date: Option<&str>) -> SearchResult {
    SearchResult {
        title: title.to_owned(),
        url: url.to_owned(),
        snippet: snippet.to_owned(),
        date: date.map(str::to_owned),
        last_updated: None,
        extra: Object::new(),
    }
}

#[test]
fn firecrawl_search_settings() {
    let search = load(&[]);
    assert!(search.api_key.is_none());
    assert_eq!(search.api_base, FIRECRAWL_API_BASE);
}

#[rstest]
#[case::key_and_base(
    &[("FIRECRAWL_API_KEY", "fc-test-key"), ("FIRECRAWL_API_BASE", "https://fc.test/v2")],
    Some("fc-test-key"),
    "https://fc.test/v2"
)]
#[case::empty_key_is_unset(&[("FIRECRAWL_API_KEY", "")], None, FIRECRAWL_API_BASE)]
#[case::empty_base_is_the_default(&[("FIRECRAWL_API_BASE", "")], None, FIRECRAWL_API_BASE)]
#[case::whitespace_key_is_a_value(&[("FIRECRAWL_API_KEY", " ")], Some(" "), FIRECRAWL_API_BASE)]
#[case::unrelated_variables_are_ignored(&[("OTHER", "x")], None, FIRECRAWL_API_BASE)]
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
        <FirecrawlSearch as SearchProvider>::ui_friendly_name(),
        "Firecrawl"
    );
    assert_eq!(load(&[]).http_method(), HttpMethod::Post);
}

#[test]
fn headers_carry_bearer_and_content_type() {
    let search = load(&[("FIRECRAWL_API_KEY", "fc-server")]);
    let prepared = search
        .prepare_headers(headers(&[("X-Trace", "1")]), None, None)
        .unwrap();
    assert_eq!(
        prepared,
        headers(&[
            ("Authorization", "Bearer fc-server"),
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
    assert!(matches!(error, FirecrawlError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "FIRECRAWL_API_KEY is not set. Set `FIRECRAWL_API_KEY` environment variable."
    );
}

#[test]
fn caller_key_skips_the_trust_check() {
    let search = load(&[("FIRECRAWL_API_KEY", "fc-server")]);
    let prepared = search
        .prepare_headers(
            Headers::new(),
            Some(&key("fc-caller")),
            Some("https://evil.test/search"),
        )
        .unwrap();
    assert_eq!(
        prepared.get("Authorization").map(String::as_str),
        Some("Bearer fc-caller")
    );
}

#[test]
fn empty_caller_key_falls_back_to_the_server_key() {
    let search = load(&[("FIRECRAWL_API_KEY", "fc-server")]);
    let prepared = search
        .prepare_headers(Headers::new(), Some(&key("")), None)
        .unwrap();
    assert_eq!(
        prepared.get("Authorization").map(String::as_str),
        Some("Bearer fc-server")
    );
}

#[test]
fn server_key_refuses_an_untrusted_caller_base() {
    let search = load(&[("FIRECRAWL_API_KEY", "fc-server")]);
    let error = search
        .prepare_headers(Headers::new(), None, Some("https://evil.test/search"))
        .unwrap_err();
    assert!(matches!(
        error,
        FirecrawlError::Credential(CredentialError::UntrustedBase { .. })
    ));
    assert!(!error.to_string().contains("fc-server"));
}

#[rstest]
#[case::default_host("https://api.firecrawl.dev/v2", &[], true)]
#[case::operator_override(
    "https://proxy.internal:9000/firecrawl",
    &[("FIRECRAWL_API_BASE", "https://proxy.internal:9000")],
    true
)]
#[case::other_host("https://api.firecrawl.dev.evil.test/search", &[], false)]
#[case::other_port("https://api.firecrawl.dev:8443/search", &[], false)]
fn server_key_trust_follows_the_configured_hosts(
    #[case] api_base: &str,
    #[case] environment: &[(&str, &str)],
    #[case] trusted: bool,
) {
    let mut settings = vec![("FIRECRAWL_API_KEY", "fc-server")];
    settings.extend_from_slice(environment);
    let prepared = load(&settings).prepare_headers(Headers::new(), None, Some(api_base));
    assert_eq!(prepared.is_ok(), trusted, "{prepared:?}");
}

#[rstest]
#[case::default_base(None, &[], "https://api.firecrawl.dev/v2/search")]
#[case::caller_base(Some("https://proxy.test"), &[], "https://proxy.test/search")]
#[case::env_base(
    None,
    &[("FIRECRAWL_API_BASE", "https://env.test/v2")],
    "https://env.test/v2/search"
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
    let search = load(&[("FIRECRAWL_API_KEY", "fc-env")]);
    let validated = search.validate_environment(headers(&[("X-Trace", "1")]), None, None);
    assert_eq!(
        validated,
        headers(&[
            ("Authorization", "Bearer fc-env"),
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
        Some("Bearer fc-env")
    );
    let keyless = load(&[]).validate_environment(headers(&[("X-Trace", "1")]), None, None);
    assert_eq!(keyless, headers(&[("X-Trace", "1")]));
    let response = ProviderResponse {
        status_code: 200,
        headers: &Headers::new(),
        text: r#"{"data":{"web":[{"title":"T","url":"https://e.test","markdown":"md"}]}}"#,
    };
    let parsed = search.transform_search_response(&response).unwrap();
    assert_eq!(
        parsed.results,
        vec![result("T", "https://e.test", "md", None)]
    );
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

fn empty_body(search: &FirecrawlSearch) -> SearchBody {
    search
        .transform_search_request(&SearchQuery::Single("rust".to_owned()), &Object::new())
        .unwrap()
}

#[test]
fn trait_sign_request_returns_auth_headers() {
    let search = load(&[("FIRECRAWL_API_KEY", "fc-env")]);
    let params = Object::new();
    let body = empty_body(&search);
    let request = outbound("https://api.firecrawl.dev/v2/search", None, &params, &body);
    let (signed, signed_body) = search.sign_request(Headers::new(), &request).unwrap();
    assert_eq!(
        signed.get("Authorization").map(String::as_str),
        Some("Bearer fc-env")
    );
    assert!(signed_body.is_none());
}

#[test]
fn trait_sign_request_reports_a_missing_key() {
    let search = load(&[]);
    let params = Object::new();
    let body = empty_body(&search);
    let request = outbound("https://api.firecrawl.dev/v2/search", None, &params, &body);
    assert!(matches!(
        search.sign_request(Headers::new(), &request),
        Err(FirecrawlError::MissingApiKey)
    ));
}

#[test]
fn trait_sign_request_sends_the_caller_key_to_an_untrusted_base() {
    let search = load(&[("FIRECRAWL_API_KEY", "fc-server")]);
    let caller = key("fc-caller");
    let params = Object::new();
    let body = empty_body(&search);
    let request = outbound("https://evil.test/search", Some(&caller), &params, &body);
    let (signed, signed_body) = search.sign_request(Headers::new(), &request).unwrap();
    assert_eq!(
        signed.get("Authorization").map(String::as_str),
        Some("Bearer fc-caller")
    );
    assert!(signed_body.is_none());
}

#[test]
fn trait_sign_request_rejects_an_untrusted_base() {
    let search = load(&[("FIRECRAWL_API_KEY", "fc-server")]);
    let params = Object::new();
    let body = empty_body(&search);
    let request = outbound("https://evil.test/search", None, &params, &body);
    let error = search.sign_request(Headers::new(), &request).unwrap_err();
    assert!(matches!(
        error,
        FirecrawlError::Credential(CredentialError::UntrustedBase { .. })
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
fn body_maps_limit_and_keeps_country_as_given() {
    let optional_params = params(
        r#"{"max_results":7,"country":"US","search_domain_filter":["docs.rs"],"max_tokens_per_page":512,"sources":["web","news"],"timeout":30000}"#,
    );
    let body = load(&[])
        .transform_search_request(&SearchQuery::Single("rust".to_owned()), &optional_params)
        .unwrap();
    assert_eq!(
        body_object(&body),
        params(
            r#"{"query":"rust","limit":7,"country":"US","sources":["web","news"],"timeout":30000,"scrapeOptions":{"formats":["markdown"],"onlyMainContent":true}}"#
        )
    );
}

#[test]
fn body_keeps_a_lowercase_country_as_given() {
    let optional_params = params(r#"{"country":"us"}"#);
    let body = load(&[])
        .transform_search_request(&SearchQuery::Single("q".to_owned()), &optional_params)
        .unwrap();
    assert_eq!(
        body_object(&body),
        params(
            r#"{"query":"q","country":"us","scrapeOptions":{"formats":["markdown"],"onlyMainContent":true}}"#
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
    let optional_params = params(r#"{"max_results":3,"limit":9,"country":"US","query":"x"}"#);
    let body = load(&[])
        .transform_search_request(&SearchQuery::Single("rust".to_owned()), &optional_params)
        .unwrap();
    assert_eq!(
        body_object(&body),
        params(
            r#"{"query":"rust","limit":3,"country":"US","scrapeOptions":{"formats":["markdown"],"onlyMainContent":true}}"#
        )
    );
}

#[test]
fn reply_reads_the_v1_flat_list() {
    let parsed = decode_firecrawl_reply(
        br#"{"success":true,"data":[
            {"title":"A","url":"https://a.test","markdown":"body","description":"desc"},
            {"title":"B","url":"https://b.test","description":"fallback"}
        ]}"#,
    )
    .unwrap();
    assert_eq!(parsed.object.0, "search");
    assert_eq!(
        parsed.results,
        vec![
            result("A", "https://a.test", "body", None),
            result("B", "https://b.test", "fallback", None),
        ]
    );
}

#[test]
fn reply_reads_web_before_news_and_keeps_the_news_date() {
    let parsed = decode_firecrawl_reply(
        br#"{"data":{
            "news":[{"title":"N","url":"https://n.test","snippet":"ns","date":"2024-06-01"}],
            "web":[{"title":"W","url":"https://w.test","markdown":"wm"}]
        }}"#,
    )
    .unwrap();
    assert_eq!(
        parsed.results,
        vec![
            result("W", "https://w.test", "wm", None),
            result("N", "https://n.test", "ns", Some("2024-06-01")),
        ]
    );
}

#[test]
fn news_markdown_wins_over_snippet() {
    let parsed = decode_firecrawl_reply(
        br#"{"data":{"news":[{"title":"N","url":"https://n.test","markdown":"md","snippet":"ns"}]}}"#,
    )
    .unwrap();
    assert_eq!(
        parsed.results,
        vec![result("N", "https://n.test", "md", None)]
    );
}

#[test]
fn news_empty_markdown_falls_back_to_snippet() {
    let parsed = decode_firecrawl_reply(
        br#"{"data":{"news":[{"title":"N","url":"https://n.test","markdown":"","snippet":"ns"}]}}"#,
    )
    .unwrap();
    assert_eq!(
        parsed.results,
        vec![result("N", "https://n.test", "ns", None)]
    );
}

#[test]
fn web_uses_description_and_ignores_a_caller_snippet() {
    let parsed = decode_firecrawl_reply(
        br#"{"data":{"web":[{"title":"W","url":"https://w.test","snippet":"ignored","description":"wd"}]}}"#,
    )
    .unwrap();
    assert_eq!(
        parsed.results,
        vec![result("W", "https://w.test", "wd", None)]
    );
}

#[test]
fn list_and_web_items_ignore_dates_but_news_keeps_its_date() {
    let list = decode_firecrawl_reply(
        br#"{"data":[{"title":"A","url":"https://a.test","date":"2024-01-02","last_updated":"2024-02-03"}]}"#,
    )
    .unwrap();
    assert_eq!(list.results, vec![result("A", "https://a.test", "", None)]);
    assert!(list.results[0].last_updated.is_none());
    let cloud = decode_firecrawl_reply(
        br#"{"data":{"web":[{"title":"W","url":"https://w.test","date":"2024-01-02","last_updated":"2024-02-03"}],"news":[{"title":"N","url":"https://n.test","snippet":"s","date":"2024-05-06","last_updated":"2024-06-07"}]}}"#,
    )
    .unwrap();
    assert_eq!(cloud.results[0], result("W", "https://w.test", "", None));
    assert!(cloud.results[0].date.is_none());
    assert!(cloud.results[0].last_updated.is_none());
    assert_eq!(
        cloud.results[1],
        result("N", "https://n.test", "s", Some("2024-05-06"))
    );
    assert!(cloud.results[1].last_updated.is_none());
}

#[test]
fn news_without_a_date_keeps_null() {
    let parsed = decode_firecrawl_reply(
        br#"{"data":{"news":[{"title":"N","url":"https://n.test","snippet":"s"}]}}"#,
    )
    .unwrap();
    assert!(parsed.results[0].date.is_none());
    let null_date = decode_firecrawl_reply(
        br#"{"data":{"news":[{"title":"N","url":"https://n.test","snippet":"s","date":null}]}}"#,
    )
    .unwrap();
    assert!(null_date.results[0].date.is_none());
}

#[test]
fn reply_defaults_missing_members_to_empty_strings() {
    let parsed = decode_firecrawl_reply(br#"{"data":{"web":[{}],"news":[{}]}}"#).unwrap();
    assert_eq!(
        parsed.results,
        vec![result("", "", "", None), result("", "", "", None)]
    );
    assert_eq!(
        serde_json::to_string(&parsed).unwrap(),
        r#"{"results":[{"title":"","url":"","snippet":"","date":null,"last_updated":null},{"title":"","url":"","snippet":"","date":null,"last_updated":null}],"object":"search"}"#
    );
}

#[rstest]
#[case::empty_object(b"{\"data\":{}}")]
#[case::empty_web(br#"{"data":{"web":[]}}"#)]
#[case::null_data(br#"{"data":null}"#)]
#[case::string_data(br#"{"data":"text"}"#)]
#[case::numeric_data(br#"{"data":5}"#)]
#[case::boolean_data(br#"{"data":true}"#)]
#[case::missing_data(br#"{"success":true}"#)]
fn reply_ignores_data_that_is_neither_a_list_nor_an_object(#[case] bytes: &[u8]) {
    let parsed = decode_firecrawl_reply(bytes).unwrap();
    assert!(parsed.results.is_empty());
    assert_eq!(parsed.object.0, "search");
}

fn rejection_kind(error: &FirecrawlError) -> &'static str {
    match error {
        FirecrawlError::Reply { .. } => "reply",
        FirecrawlError::Result { .. } => "result",
        FirecrawlError::NotText { .. } => "text",
        FirecrawlError::TrailingBytes(_) => "trailing",
        FirecrawlError::Credential(_) => "credential",
        FirecrawlError::MissingApiKey => "missing key",
        FirecrawlError::ReplyTooLarge { .. } => "too large",
    }
}

#[rstest]
#[case::array_document(b"[]", "reply")]
#[case::nested_array_document(br#"[[{"title":"x"}]]"#, "reply")]
#[case::string_document(br#""text""#, "reply")]
#[case::web_null(br#"{"data":{"web":null}}"#, "result")]
#[case::news_null(br#"{"data":{"news":null}}"#, "result")]
#[case::web_not_a_list(br#"{"data":{"web":{}}}"#, "result")]
#[case::item_not_an_object(br#"{"data":[null]}"#, "result")]
#[case::null_title(br#"{"data":[{"title":null}]}"#, "text")]
#[case::numeric_url(br#"{"data":[{"url":7}]}"#, "text")]
#[case::null_description_without_markdown(br#"{"data":[{"description":null}]}"#, "text")]
#[case::null_snippet_without_markdown(br#"{"data":{"news":[{"snippet":null}]}}"#, "text")]
#[case::numeric_news_date(br#"{"data":{"news":[{"date":7}]}}"#, "text")]
fn reply_rejects_a_shape_the_source_would_fail(#[case] bytes: &[u8], #[case] kind: &str) {
    let error = decode_firecrawl_reply(bytes).unwrap_err();
    assert_eq!(rejection_kind(&error), kind);
}

#[test]
fn reply_rejects_invalid_json() {
    assert!(matches!(
        decode_firecrawl_reply(b"not json"),
        Err(FirecrawlError::Reply { .. })
    ));
}

#[test]
fn reply_rejects_trailing_bytes() {
    assert!(matches!(
        decode_firecrawl_reply(br#"{"data":[]}x"#),
        Err(FirecrawlError::TrailingBytes(_))
    ));
}

#[test]
fn reply_is_bounded() {
    let mut bytes = vec![b' '; MAX_REPLY_BYTES + 1];
    assert!(matches!(
        decode_firecrawl_reply(&bytes),
        Err(FirecrawlError::ReplyTooLarge {
            limit: MAX_REPLY_BYTES
        })
    ));
    bytes.truncate(MAX_REPLY_BYTES);
    assert!(matches!(
        decode_firecrawl_reply(&bytes),
        Err(FirecrawlError::Reply { .. })
    ));
}

#[rstest]
fn replays_the_firecrawl_corpus(#[files("fuzz/corpus/firecrawl_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(parsed) = decode_firecrawl_reply(&bytes) {
        assert_eq!(parsed.object.0, "search");
        assert!(
            parsed
                .results
                .iter()
                .all(|result| result.last_updated.is_none())
        );
    }
}
