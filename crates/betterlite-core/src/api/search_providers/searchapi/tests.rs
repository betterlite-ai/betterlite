#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str;

use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{HttpMethod, SearchQuery};
use rstest::rstest;
use secrecy::SecretString;

use super::{SearchApiError, SearchApiSearch, decode_reply};
use crate::api::search_providers::google_pse::wire::SearchParam;

const DEFAULT_BASE: &str = "https://www.searchapi.io/api/v1/search";

fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn provider(lookup: &BTreeMap<String, String>) -> SearchApiSearch<'_> {
    SearchApiSearch {
        lookup,
        api_key: Some(SecretString::from("caller-key")),
        api_base: None,
    }
}

fn params(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

fn single(query: &str) -> SearchQuery {
    SearchQuery::Single(query.to_owned())
}

#[test]
fn display_name_and_method_match_litellm() {
    assert_eq!(
        SearchApiSearch::UI_FRIENDLY_NAME,
        "SearchAPI.io (Google Search)"
    );
    assert_eq!(SearchApiSearch::HTTP_METHOD, HttpMethod::Get);
}

#[test]
fn validate_environment_adds_only_content_type() {
    let lookup = environment(&[]);
    let sent = Headers::from([(String::from("x-extra"), String::from("kept"))]);

    let headers = provider(&lookup).validate_environment(sent).unwrap();

    assert_eq!(
        headers,
        Headers::from([
            (
                String::from("Content-Type"),
                String::from("application/json")
            ),
            (String::from("x-extra"), String::from("kept")),
        ])
    );
}

#[test]
fn validate_environment_reports_the_missing_key_text() {
    let lookup = environment(&[]);
    let mut search = provider(&lookup);
    search.api_key = None;

    let error = search.validate_environment(Headers::new()).unwrap_err();

    assert_eq!(
        error.to_string(),
        "SEARCHAPI_API_KEY is not set. Set `SEARCHAPI_API_KEY` environment variable."
    );
}

#[test]
fn transform_reports_the_missing_key_text() {
    let lookup = environment(&[]);
    let mut search = provider(&lookup);
    search.api_key = None;

    let error = search
        .transform_search_request(&single("rust"), &Object::new())
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "SEARCHAPI_API_KEY is not set. Set `SEARCHAPI_API_KEY` environment variable."
    );
}

#[test]
fn transform_keeps_the_source_record_order_in_the_query_string() {
    let lookup = environment(&[]);
    let options = params(
        r#"{"max_results": 25, "search_domain_filter": ["a.example", "b.example"], "country": "US", "page": 2}"#,
    );

    let body = provider(&lookup)
        .transform_search_request(&single("rust"), &options)
        .unwrap();
    let url = provider(&lookup).complete_url(None, Some(&body)).unwrap();

    assert_eq!(
        url,
        format!(
            "{DEFAULT_BASE}?engine=google&q=%28rust%29+AND+%28site%3Aa.example+OR+site%3Ab.example%29&api_key=caller-key&num=10&gl=us&page=2"
        )
    );
}

#[rstest]
#[case::above_cap(15, 10)]
#[case::at_cap(10, 10)]
#[case::below_cap(5, 5)]
#[case::negative_kept(-3, -3)]
fn max_results_is_capped_above_without_a_floor(#[case] requested: i64, #[case] expected: i64) {
    let lookup = environment(&[]);
    let options = params(&format!(r#"{{"max_results": {requested}}}"#));

    let body = provider(&lookup)
        .transform_search_request(&single("rust"), &options)
        .unwrap();

    assert_eq!(body.params.get("num"), Some(&SearchParam::Number(expected)));
}

#[test]
fn max_results_absent_sends_no_num() {
    let lookup = environment(&[]);

    let body = provider(&lookup)
        .transform_search_request(&single("rust"), &Object::new())
        .unwrap();

    assert!(!body.params.contains_key("num"));
}

#[test]
fn domain_list_rewrites_the_query_with_and_and_or() {
    let lookup = environment(&[]);
    let options = params(r#"{"search_domain_filter": ["a.example", "b.example", "c.example"]}"#);

    let body = provider(&lookup)
        .transform_search_request(&single("alpha beta"), &options)
        .unwrap();

    assert_eq!(
        body.params.get("q"),
        Some(&SearchParam::Text(String::from(
            "(alpha beta) AND (site:a.example OR site:b.example OR site:c.example)"
        )))
    );
}

#[rstest]
#[case::empty_list(r#"{"search_domain_filter": []}"#, "rust")]
#[case::string_ignored(r#"{"search_domain_filter": "a.example"}"#, "rust")]
#[case::absent("{}", "rust")]
fn a_query_is_not_rewritten_without_a_domain_list(#[case] json: &str, #[case] expected: &str) {
    let lookup = environment(&[]);

    let body = provider(&lookup)
        .transform_search_request(&single("rust"), &params(json))
        .unwrap();

    assert_eq!(
        body.params.get("q"),
        Some(&SearchParam::Text(expected.to_owned()))
    );
}

#[test]
fn country_is_lowercased_into_gl() {
    let lookup = environment(&[]);
    let options = params(r#"{"country": "DE"}"#);

    let body = provider(&lookup)
        .transform_search_request(&single("rust"), &options)
        .unwrap();

    assert_eq!(
        body.params.get("gl"),
        Some(&SearchParam::Text(String::from("de")))
    );
}

#[test]
fn unified_max_tokens_per_page_is_dropped() {
    let lookup = environment(&[]);
    let options = params(r#"{"max_tokens_per_page": 512}"#);

    let body = provider(&lookup)
        .transform_search_request(&single("rust"), &options)
        .unwrap();

    assert!(!body.params.contains_key("max_tokens_per_page"));
}

#[test]
fn passthrough_never_overwrites_a_mapped_field() {
    let lookup = environment(&[]);
    let options = params(r#"{"engine": "bing", "api_key": "other", "q": "other"}"#);

    let body = provider(&lookup)
        .transform_search_request(&single("rust"), &options)
        .unwrap();

    assert_eq!(
        body.params.get("engine"),
        Some(&SearchParam::Text(String::from("google")))
    );
    assert_eq!(
        body.params.get("q"),
        Some(&SearchParam::Text(String::from("rust")))
    );
    assert_eq!(
        body.params.get("api_key"),
        Some(&SearchParam::Text(String::from("caller-key")))
    );
}

#[test]
fn a_query_list_is_joined_with_spaces() {
    let lookup = environment(&[]);

    let body = provider(&lookup)
        .transform_search_request(
            &SearchQuery::Multiple(vec![String::from("alpha"), String::from("beta")]),
            &Object::new(),
        )
        .unwrap();

    assert_eq!(
        body.params.get("q"),
        Some(&SearchParam::Text(String::from("alpha beta")))
    );
}

#[test]
fn a_list_option_is_encoded_with_doseq() {
    let lookup = environment(&[]);
    let options = params(r#"{"location": ["Paris", "Lyon"]}"#);

    let body = provider(&lookup)
        .transform_search_request(&single("rust"), &options)
        .unwrap();
    let url = provider(&lookup).complete_url(None, Some(&body)).unwrap();

    assert_eq!(
        url,
        format!(
            "{DEFAULT_BASE}?engine=google&q=rust&api_key=caller-key&location=Paris&location=Lyon"
        )
    );
}

#[rstest]
#[case::tilde_kept_star_encoded("a~b*c", "q=a~b%2Ac")]
#[case::reserved_encoded("a&b=c/d", "q=a%26b%3Dc%2Fd")]
#[case::space_is_plus("a b", "q=a+b")]
#[case::unicode_utf8("\u{e9}", "q=%C3%A9")]
fn query_string_quotes_like_python_quote_plus(#[case] query: &str, #[case] expected: &str) {
    let lookup = environment(&[]);

    let body = provider(&lookup)
        .transform_search_request(&single(query), &Object::new())
        .unwrap();
    let url = provider(&lookup).complete_url(None, Some(&body)).unwrap();

    assert_eq!(
        url,
        format!("{DEFAULT_BASE}?engine=google&{expected}&api_key=caller-key")
    );
}

#[rstest]
#[case::caller_base(Some("https://proxy.test/v1"), &[("SEARCHAPI_API_BASE", "https://env.test")], "https://proxy.test/v1")]
#[case::environment_base(None, &[("SEARCHAPI_API_BASE", "https://env.test")], "https://env.test")]
#[case::empty_caller_base(None, &[("SEARCHAPI_API_BASE", "")], DEFAULT_BASE)]
#[case::default_base(None, &[], DEFAULT_BASE)]
fn complete_url_prefers_the_caller_then_the_environment(
    #[case] api_base: Option<&str>,
    #[case] entries: &[(&str, &str)],
    #[case] expected: &str,
) {
    let lookup = environment(entries);

    let url = provider(&lookup).complete_url(api_base, None).unwrap();

    assert_eq!(url, expected);
}

#[test]
fn an_environment_key_cannot_reach_a_caller_chosen_host() {
    let lookup = environment(&[("SEARCHAPI_API_KEY", "server-key")]);
    let mut search = provider(&lookup);
    search.api_key = None;
    search.api_base = Some(String::from("https://evil.example/v1"));

    let error = search.validate_environment(Headers::new()).unwrap_err();

    assert!(matches!(error, SearchApiError::Credential(_)));
    assert!(!error.to_string().contains("server-key"));
}

#[test]
fn the_second_resolution_repeats_the_host_check() {
    let lookup = environment(&[("SEARCHAPI_API_KEY", "server-key")]);
    let mut search = provider(&lookup);
    search.api_key = None;
    search.api_base = Some(String::from("https://evil.example/v1"));

    let error = search
        .transform_search_request(&single("rust"), &Object::new())
        .unwrap_err();

    assert!(matches!(error, SearchApiError::Credential(_)));
    assert!(!error.to_string().contains("server-key"));
}

#[rstest]
#[case::default_host("https://www.searchapi.io/other/path", &[])]
#[case::operator_override("https://proxy.internal:9000/search", &[("SEARCHAPI_API_BASE", "https://proxy.internal:9000")])]
fn an_environment_key_reaches_a_trusted_host(
    #[case] api_base: &str,
    #[case] entries: &[(&str, &str)],
) {
    let mut all = vec![("SEARCHAPI_API_KEY", "server-key")];
    all.extend_from_slice(entries);
    let lookup = environment(&all);
    let mut search = provider(&lookup);
    search.api_key = None;
    search.api_base = Some(api_base.to_owned());

    let body = search
        .transform_search_request(&single("rust"), &Object::new())
        .unwrap();

    assert_eq!(
        body.params.get("api_key"),
        Some(&SearchParam::Text(String::from("server-key")))
    );
}

#[test]
fn an_explicit_key_bypasses_the_host_check() {
    let lookup = environment(&[]);
    let mut search = provider(&lookup);
    search.api_base = Some(String::from("https://evil.example/v1"));

    let body = search
        .transform_search_request(&single("rust"), &Object::new())
        .unwrap();

    assert_eq!(
        body.params.get("api_key"),
        Some(&SearchParam::Text(String::from("caller-key")))
    );
}

#[test]
fn a_reply_without_organic_results_gives_no_results() {
    let response = decode_reply(r#"{"search_metadata": {"status": "Success"}}"#).unwrap();

    assert!(response.results.is_empty());
    assert_eq!(response.object.0, "search");
}

#[test]
fn an_organic_result_projects_title_link_snippet_and_date() {
    let response = decode_reply(
        r#"{"organic_results": [{"title": "Rust", "link": "https://rust.example", "snippet": "A language.", "date": "Mar 1, 2026", "position": 1}]}"#,
    )
    .unwrap();

    let result = response.results.first().unwrap();
    assert_eq!(result.title, "Rust");
    assert_eq!(result.url, "https://rust.example");
    assert_eq!(result.snippet, "A language.");
    assert_eq!(result.date.as_deref(), Some("Mar 1, 2026"));
    assert_eq!(result.last_updated, None);
}

#[test]
fn an_organic_result_without_a_date_keeps_it_null() {
    let response = decode_reply(r#"{"organic_results": [{"title": "Rust"}]}"#).unwrap();

    let result = response.results.first().unwrap();
    assert_eq!(result.url, "");
    assert_eq!(result.snippet, "");
    assert_eq!(result.date, None);
}

#[rstest]
#[case::null_title(r#"{"organic_results": [{"title": null}]}"#)]
#[case::numeric_date(r#"{"organic_results": [{"date": 7}]}"#)]
#[case::results_null(r#"{"organic_results": null}"#)]
#[case::not_json("not json")]
#[case::results_not_a_list(r#"{"organic_results": {}}"#)]
fn a_malformed_reply_is_an_error(#[case] text: &str) {
    drop(decode_reply(text).unwrap_err());
}

#[test]
fn debug_never_shows_the_key() {
    let lookup = environment(&[]);

    let rendered = format!("{:?}", provider(&lookup));

    assert!(!rendered.contains("caller-key"));
    assert!(rendered.contains("SearchApiSearch"));
    assert!(rendered.contains("[REDACTED]"));
}

#[test]
fn a_transformed_body_debug_hides_the_key() {
    let lookup = environment(&[]);
    let body = provider(&lookup)
        .transform_search_request(&single("rust"), &Object::new())
        .unwrap();

    let rendered = format!("{body:?}");

    assert!(!rendered.contains("caller-key"));
    assert!(rendered.contains("[REDACTED]"));
    assert!(rendered.contains("rust"));
}

#[rstest]
fn replays_the_searchapi_corpus(#[files("fuzz/corpus/searchapi_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    drop(decode_reply(text));
}
