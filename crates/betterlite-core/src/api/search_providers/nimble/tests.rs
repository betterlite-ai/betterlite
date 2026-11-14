#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::CredentialError;
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::error::NimbleSearchError;
use super::reply::{NimbleReplyError, decode_nimble_reply};
use super::request::SearchBody;
use super::{NIMBLE_API_BASE, NIMBLE_DOCS_URL, NimbleSearch};

fn load(environment: &[(&str, &str)]) -> NimbleSearch {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    NimbleSearch::load(&lookup)
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

fn serialized(body: &SearchBody) -> String {
    serde_json::to_string(body).unwrap()
}

fn decode(text: &str) -> Result<SearchResponse, NimbleReplyError> {
    decode_nimble_reply(text.as_bytes())
}

fn single_body(options: &str) -> SearchBody {
    load(&[])
        .transform_search_request(&SearchQuery::Single("rust".to_owned()), &params(options))
        .unwrap()
}

#[test]
fn display_name_method_and_rich_input_match_the_source() {
    assert_eq!(
        <NimbleSearch as SearchProvider>::ui_friendly_name(),
        "Nimble"
    );
    let search = load(&[]);
    assert_eq!(search.http_method(), HttpMethod::Post);
    assert!(!search.supports_rich_search_input());
}

#[rstest]
#[case::default(&[], None, NIMBLE_API_BASE)]
#[case::environment(
    &[("NIMBLE_API_KEY", "nimble-server"), ("NIMBLE_API_BASE", "https://env.test/v2")],
    Some("nimble-server"),
    "https://env.test/v2"
)]
#[case::empty_variables(&[("NIMBLE_API_KEY", ""), ("NIMBLE_API_BASE", "")], None, NIMBLE_API_BASE)]
fn settings_load_from_the_environment(
    #[case] environment: &[(&str, &str)],
    #[case] api_key: Option<&str>,
    #[case] api_base: &str,
) {
    let search = load(environment);
    assert_eq!(
        search.api_key.as_ref().map(ExposeSecret::expose_secret),
        api_key
    );
    assert_eq!(search.api_base, api_base);
}

#[test]
fn headers_carry_bearer_client_source_and_content_type() {
    let search = load(&[("NIMBLE_API_KEY", "nimble-server")]);
    let prepared = search
        .prepare_headers(headers(&[("X-Trace", "1")]), None, None)
        .unwrap();
    assert_eq!(
        prepared,
        headers(&[
            ("Authorization", "Bearer nimble-server"),
            ("Content-Type", "application/json"),
            ("X-Client-Source", "litellm"),
            ("X-Trace", "1"),
        ])
    );
}

#[test]
fn missing_key_names_the_variable() {
    let error = load(&[])
        .prepare_headers(Headers::new(), None, None)
        .unwrap_err();
    assert!(matches!(error, NimbleSearchError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "NIMBLE_API_KEY is not set. Set `NIMBLE_API_KEY` environment variable."
    );
}

#[test]
fn caller_key_skips_the_trust_check() {
    let search = load(&[("NIMBLE_API_KEY", "nimble-server")]);
    let prepared = search
        .prepare_headers(
            Headers::new(),
            Some(&key("nimble-caller")),
            Some("https://evil.test/search"),
        )
        .unwrap();
    assert_eq!(
        prepared.get("Authorization").map(String::as_str),
        Some("Bearer nimble-caller")
    );
}

#[test]
fn empty_caller_key_falls_back_to_the_server_key() {
    let search = load(&[("NIMBLE_API_KEY", "nimble-server")]);
    let prepared = search
        .prepare_headers(Headers::new(), Some(&key("")), None)
        .unwrap();
    assert_eq!(
        prepared.get("Authorization").map(String::as_str),
        Some("Bearer nimble-server")
    );
}

#[test]
fn server_key_refuses_an_untrusted_caller_base() {
    let search = load(&[("NIMBLE_API_KEY", "nimble-server")]);
    let error = search
        .prepare_headers(Headers::new(), None, Some("https://evil.test/search"))
        .unwrap_err();
    assert!(matches!(
        error,
        NimbleSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
    assert!(!error.to_string().contains("nimble-server"));
}

#[rstest]
#[case::default_host("https://sdk.nimbleway.com/v2/search", &[], true)]
#[case::operator_override(
    "https://proxy.internal:9000/nimble",
    &[("NIMBLE_API_BASE", "https://proxy.internal:9000")],
    true
)]
#[case::other_host("https://sdk.nimbleway.com.evil.test/search", &[], false)]
#[case::other_port("https://sdk.nimbleway.com:8443/search", &[], false)]
fn server_key_trust_follows_the_configured_hosts(
    #[case] api_base: &str,
    #[case] environment: &[(&str, &str)],
    #[case] trusted: bool,
) {
    let mut settings = vec![("NIMBLE_API_KEY", "nimble-server")];
    settings.extend_from_slice(environment);
    let prepared = load(&settings).prepare_headers(Headers::new(), None, Some(api_base));
    assert_eq!(prepared.is_ok(), trusted, "{prepared:?}");
}

#[rstest]
#[case::default_base(None, &[], "https://sdk.nimbleway.com/v2/search")]
#[case::caller_base(Some("https://proxy.test"), &[], "https://proxy.test/search")]
#[case::env_base(
    None,
    &[("NIMBLE_API_BASE", "https://env.test/v2/")],
    "https://env.test/v2/search"
)]
#[case::existing_suffix(Some("https://proxy.test/search"), &[], "https://proxy.test/search")]
#[case::trailing_slash_on_suffix(
    Some("https://proxy.test/search/"),
    &[],
    "https://proxy.test/search"
)]
#[case::double_slash(Some("https://proxy.test/"), &[], "https://proxy.test/search")]
#[case::lookalike_suffix(
    Some("https://proxy.test/mysearch"),
    &[],
    "https://proxy.test/mysearch/search"
)]
#[case::empty_caller_base_is_used_as_given(Some(""), &[], "/search")]
fn endpoint_trims_trailing_slashes_and_appends_search(
    #[case] api_base: Option<&str>,
    #[case] environment: &[(&str, &str)],
    #[case] expected: &str,
) {
    assert_eq!(load(environment).endpoint(api_base), expected);
}

#[test]
fn trait_hooks_shape_headers_and_url() {
    let search = load(&[("NIMBLE_API_KEY", "nimble-env")]);
    let validated = search.validate_environment(headers(&[("X-Trace", "1")]), None, None);
    assert_eq!(
        validated.get("X-Client-Source").map(String::as_str),
        Some("litellm")
    );
    assert_eq!(
        search
            .complete_url(Some("https://proxy.test/"), &Object::new(), None)
            .unwrap(),
        "https://proxy.test/search"
    );
    let keyless = load(&[]).validate_environment(headers(&[("X-Trace", "1")]), None, None);
    assert_eq!(keyless, headers(&[("X-Trace", "1")]));
    let empty_caller_key = search.validate_environment(Headers::new(), Some(&key("")), None);
    assert_eq!(
        empty_caller_key.get("Authorization").map(String::as_str),
        Some("Bearer nimble-env")
    );
    let caller_key = search.validate_environment(Headers::new(), Some(&key("nimble-caller")), None);
    assert_eq!(
        caller_key.get("Authorization").map(String::as_str),
        Some("Bearer nimble-caller")
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

#[test]
fn sign_request_returns_auth_headers() {
    let search = load(&[("NIMBLE_API_KEY", "nimble-env")]);
    let params = Object::new();
    let body = single_body("{}");
    let request = outbound("https://sdk.nimbleway.com/v2/search", None, &params, &body);
    let (signed, signed_body) = search.sign_request(Headers::new(), &request).unwrap();
    assert_eq!(
        signed.get("Authorization").map(String::as_str),
        Some("Bearer nimble-env")
    );
    assert_eq!(
        signed.get("X-Client-Source").map(String::as_str),
        Some("litellm")
    );
    assert!(signed_body.is_none());
}

#[test]
fn sign_request_reports_a_missing_key() {
    let search = load(&[]);
    let params = Object::new();
    let body = single_body("{}");
    let request = outbound("https://sdk.nimbleway.com/v2/search", None, &params, &body);
    assert!(matches!(
        search.sign_request(Headers::new(), &request),
        Err(NimbleSearchError::MissingApiKey)
    ));
}

#[test]
fn sign_request_sends_the_caller_key_to_an_untrusted_base() {
    let search = load(&[("NIMBLE_API_KEY", "nimble-server")]);
    let caller = key("nimble-caller");
    let params = Object::new();
    let body = single_body("{}");
    let request = outbound("https://evil.test/search", Some(&caller), &params, &body);
    let (signed, _signed_body) = search.sign_request(Headers::new(), &request).unwrap();
    assert_eq!(
        signed.get("Authorization").map(String::as_str),
        Some("Bearer nimble-caller")
    );
}

#[test]
fn sign_request_rejects_an_untrusted_base() {
    let search = load(&[("NIMBLE_API_KEY", "nimble-server")]);
    let params = Object::new();
    let body = single_body("{}");
    let request = outbound("https://evil.test/search", None, &params, &body);
    assert!(matches!(
        search.sign_request(Headers::new(), &request),
        Err(NimbleSearchError::Credential(
            CredentialError::UntrustedBase { .. }
        ))
    ));
}

#[test]
fn body_keeps_the_source_key_order() {
    let body = load(&[])
        .transform_search_request(
            &SearchQuery::Multiple(vec!["rust".to_owned(), "search".to_owned()]),
            &params(
                r#"{"location":"us","max_results":5,"country":"us","search_domain_filter":["news.test","-spam.test","","-","docs.test"],"max_tokens_per_page":100}"#,
            ),
        )
        .unwrap();
    assert_eq!(
        serialized(&body),
        r#"{"include_domains":["news.test","docs.test"],"exclude_domains":["spam.test"],"location":"us","query":"rust search","max_results":5,"country":"US"}"#
    );
}

#[test]
fn body_caller_domains_beat_the_derived_ones() {
    let body = single_body(
        r#"{"search_domain_filter":["news.test","-spam.test"],"include_domains":["mine.test"],"exclude_domains":["other.test"]}"#,
    );
    assert_eq!(
        serialized(&body),
        r#"{"include_domains":["mine.test"],"exclude_domains":["other.test"],"query":"rust"}"#
    );
}

#[test]
fn body_caller_query_does_not_beat_the_query() {
    assert_eq!(
        serialized(&single_body(r#"{"query":"mine"}"#)),
        r#"{"query":"rust"}"#
    );
}

#[rstest]
#[case::empty_filter(r#"{"search_domain_filter":[]}"#, r#"{"query":"rust"}"#)]
#[case::null_filter(r#"{"search_domain_filter":null}"#, r#"{"query":"rust"}"#)]
#[case::text_filter(r#"{"search_domain_filter":"news.test"}"#, r#"{"query":"rust"}"#)]
#[case::numeric_entry(r#"{"search_domain_filter":["a.test",7]}"#, r#"{"query":"rust"}"#)]
#[case::nested_entry(r#"{"search_domain_filter":[["a.test"]]}"#, r#"{"query":"rust"}"#)]
#[case::only_empty_entries(r#"{"search_domain_filter":["","-"]}"#, r#"{"query":"rust"}"#)]
#[case::split(
    r#"{"search_domain_filter":["a.test","-b.test","","-","-c-d.test"]}"#,
    r#"{"include_domains":["a.test"],"exclude_domains":["b.test","c-d.test"],"query":"rust"}"#
)]
#[case::exclude_only(
    r#"{"search_domain_filter":["-b.test"]}"#,
    r#"{"exclude_domains":["b.test"],"query":"rust"}"#
)]
fn body_splits_the_domain_filter(#[case] options: &str, #[case] expected: &str) {
    assert_eq!(serialized(&single_body(options)), expected);
}

#[rstest]
#[case::zero(r#"{"max_results":0}"#, r#"{"query":"rust","max_results":0}"#)]
#[case::above_the_limit(r#"{"max_results":500}"#, r#"{"query":"rust","max_results":500}"#)]
#[case::null_is_absent(r#"{"max_results":null}"#, r#"{"query":"rust"}"#)]
#[case::false_is_sent(r#"{"max_results":false}"#, r#"{"query":"rust","max_results":false}"#)]
#[case::country_uppercases(r#"{"country":"de"}"#, r#"{"query":"rust","country":"DE"}"#)]
#[case::country_null_is_absent(r#"{"country":null}"#, r#"{"query":"rust"}"#)]
#[case::country_number_is_absent(r#"{"country":7}"#, r#"{"query":"rust"}"#)]
#[case::empty_country_stays_empty(r#"{"country":""}"#, r#"{"query":"rust","country":""}"#)]
fn body_maps_max_results_and_country(#[case] options: &str, #[case] expected: &str) {
    assert_eq!(serialized(&single_body(options)), expected);
}

#[rstest]
#[case::content_wins(r#"{"content":"c","description":"d"}"#, "c")]
#[case::empty_content_falls_back(r#"{"content":"","description":"d"}"#, "d")]
#[case::description_falls_back(r#"{"description":"d"}"#, "d")]
#[case::nothing_is_empty(r#"{}"#, "")]
#[case::nulls_are_absent(r#"{"content":null,"description":null}"#, "")]
#[case::empty_description_is_empty(r#"{"description":""}"#, "")]
fn reply_projects_the_snippet(#[case] result: &str, #[case] snippet: &str) {
    let response = decode(&format!(r#"{{"results":[{result}]}}"#)).unwrap();
    assert_eq!(response.results[0].snippet, snippet);
}

#[test]
fn reply_projects_title_url_and_keeps_the_order() {
    let response = decode(
        r#"{"results":[{"title":"A","url":"https://a.test"},{"title":null,"url":null},{"title":"C","url":"https://c.test"}]}"#,
    )
    .unwrap();
    assert_eq!(response.object.0, "search");
    assert_eq!(
        response
            .results
            .iter()
            .map(|result| result.title.as_str())
            .collect::<Vec<_>>(),
        ["A", "", "C"]
    );
    assert_eq!(
        response
            .results
            .iter()
            .map(|result| result.url.as_str())
            .collect::<Vec<_>>(),
        ["https://a.test", "", "https://c.test"]
    );
    assert!(response.extra.is_empty());
    assert!(response.hidden_params.is_empty());
    assert!(
        response
            .results
            .iter()
            .all(|result| result.last_updated.is_none())
    );
}

#[rstest]
#[case::publish_date(
    r#"{"additional_data":{"publish_date":"2024-05-01","publish_date_raw":"1 day ago"}}"#,
    Some("2024-05-01")
)]
#[case::raw_only(r#"{"additional_data":{"publish_date_raw":"1 day ago"}}"#, None)]
#[case::numeric_publish_date(r#"{"additional_data":{"publish_date":7}}"#, None)]
#[case::null_publish_date(r#"{"additional_data":{"publish_date":null}}"#, None)]
#[case::empty_publish_date(r#"{"additional_data":{"publish_date":""}}"#, Some(""))]
#[case::text_additional_data(r#"{"additional_data":"text"}"#, None)]
#[case::list_additional_data(r#"{"additional_data":["2024"]}"#, None)]
#[case::null_additional_data(r#"{"additional_data":null}"#, None)]
#[case::missing_additional_data("{}", None)]
fn reply_reads_the_publish_date(#[case] result: &str, #[case] date: Option<&str>) {
    let response = decode(&format!(r#"{{"results":[{result}]}}"#)).unwrap();
    assert_eq!(response.results[0].date.as_deref(), date);
}

#[rstest]
#[case::object(
    r#"{"additional_data":{"publish_date":"2024","position":2}}"#,
    Some(r#"{"publish_date":"2024","position":2}"#)
)]
#[case::boolean_false(r#"{"additional_data":false}"#, Some("false"))]
#[case::number(r#"{"additional_data":0}"#, Some("0"))]
#[case::empty_text(r#"{"additional_data":""}"#, Some(r#""""#))]
#[case::null(r#"{"additional_data":null}"#, None)]
#[case::missing("{}", None)]
fn reply_keeps_additional_data_as_an_extra(#[case] result: &str, #[case] expected: Option<&str>) {
    let response = decode(&format!(r#"{{"results":[{result}]}}"#)).unwrap();
    let expected_extra = expected.map_or_else(Object::new, |value| {
        params(&format!(r#"{{"additional_data":{value}}}"#))
    });
    assert_eq!(response.results[0].extra, expected_extra);
}

#[rstest]
#[case::missing_results(r#"{}"#)]
#[case::null_results(r#"{"results":null}"#)]
#[case::results_is_an_object(r#"{"results":{}}"#)]
#[case::results_is_text(r#"{"results":"x"}"#)]
#[case::top_level_array("[]")]
#[case::top_level_text(r#""x""#)]
#[case::null_result(r#"{"results":[null]}"#)]
#[case::list_result(r#"{"results":[[]]}"#)]
#[case::numeric_title(r#"{"results":[{"title":7}]}"#)]
#[case::numeric_url(r#"{"results":[{"url":7}]}"#)]
#[case::numeric_content(r#"{"results":[{"content":7}]}"#)]
#[case::numeric_description(r#"{"results":[{"description":7}]}"#)]
#[case::invalid_json("not json")]
#[case::trailing_bytes(r#"{"results":[]}x"#)]
fn reply_rejects_a_shape_the_source_would_fail(#[case] body: &str) {
    assert!(decode(body).is_err(), "{body}");
}

#[test]
fn a_schema_failure_raises_through_the_error_class() {
    let search = load(&[]);
    let headers = headers(&[("X-Trace", "1")]);
    let response = ProviderResponse {
        status_code: 422,
        headers: &headers,
        text: r#"{"unexpected":true}"#,
    };
    match search.transform_search_response(&response).unwrap_err() {
        NimbleSearchError::Provider(error) => {
            assert_eq!(error.status_code, 422);
            assert!(
                error.message.starts_with(
                    "Nimble Search: response does not match the documented /v2/search schema: "
                ),
                "{}",
                error.message
            );
            assert!(
                error
                    .message
                    .ends_with(&format!(". See {NIMBLE_DOCS_URL} for details."))
            );
            assert_eq!(
                error.headers,
                Some(vec![("X-Trace".to_owned(), "1".to_owned())])
            );
        }
        other => panic!("expected a provider error, got {other}"),
    }
}

#[rstest]
#[case::detail(r#"{"detail":"bad query"}"#, "bad query")]
#[case::message(
    r#"{"success":"false","task_id":"t","message":"collection failed"}"#,
    "collection failed"
)]
#[case::detail_wins(r#"{"detail":"first","message":"second"}"#, "first")]
#[case::empty_detail_falls_to_message(r#"{"detail":"","message":"second"}"#, "second")]
#[case::numeric_detail_is_raw(r#"{"detail":7}"#, r#"{"detail":7}"#)]
#[case::empty_object_is_raw(r#"{}"#, "{}")]
#[case::array_document_is_raw("[1,2]", "[1,2]")]
#[case::plain_text_is_raw("upstream said no", "upstream said no")]
#[case::trailing_dots_and_spaces(r#"{"detail":"bad. . "}"#, "bad")]
#[case::plain_text_trailing("boom... ", "boom")]
fn error_text_unwraps_the_envelope(#[case] text: &str, #[case] detail: &str) {
    let error = load(&[]).error_class(text, 400, &Headers::new());
    assert_eq!(error.status_code, 400);
    assert_eq!(
        error.message,
        format!("Nimble Search: {detail}. See {NIMBLE_DOCS_URL} for details.")
    );
}

#[test]
fn an_http_error_uses_the_shaped_message() {
    let error = load(&[]).http_error(&ProviderResponse {
        status_code: 401,
        headers: &Headers::new(),
        text: r#"{"detail":"Unauthorized"}"#,
    });
    assert_eq!(error.status_code, 401);
    assert_eq!(
        error.message,
        format!("Nimble Search: Unauthorized. See {NIMBLE_DOCS_URL} for details.")
    );
}

#[rstest]
fn replays_the_nimble_corpus(#[files("fuzz/corpus/nimble_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(parsed) = decode_nimble_reply(&bytes) {
        assert_eq!(parsed.object.0, "search");
        assert!(parsed.hidden_params.is_empty());
    }
}
