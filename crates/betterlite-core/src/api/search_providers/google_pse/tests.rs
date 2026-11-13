#![cfg(test)]

use std::borrow::ToOwned;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str;

use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{HttpMethod, SearchQuery};
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::wire::SearchParam;
use super::{GooglePseError, GooglePseSearch, decode_reply};

const DEFAULT_BASE: &str = "https://www.googleapis.com/customsearch/v1";

fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn provider(lookup: &BTreeMap<String, String>) -> GooglePseSearch<'_> {
    GooglePseSearch {
        lookup,
        api_key: Some(SecretString::from("caller-key")),
        api_base: None,
        search_engine_id: Some(String::from("engine-id")),
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
    assert_eq!(GooglePseSearch::UI_FRIENDLY_NAME, "Google PSE");
    assert_eq!(GooglePseSearch::HTTP_METHOD, HttpMethod::Get);
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
fn validate_environment_reports_missing_key_with_the_source_text() {
    let lookup = environment(&[]);
    let mut search = provider(&lookup);
    search.api_key = None;

    let error = search.validate_environment(Headers::new()).unwrap_err();

    assert_eq!(
        error.to_string(),
        "GOOGLE_PSE_API_KEY is not set. Set `GOOGLE_PSE_API_KEY` environment variable."
    );
}

#[test]
fn validate_environment_reports_missing_engine_id_with_the_source_text() {
    let lookup = environment(&[("GOOGLE_PSE_API_KEY", "server-key")]);
    let mut search = provider(&lookup);
    search.api_key = None;
    search.search_engine_id = None;

    let error = search.validate_environment(Headers::new()).unwrap_err();

    assert_eq!(
        error.to_string(),
        "GOOGLE_PSE_ENGINE_ID is not set. Set `GOOGLE_PSE_ENGINE_ID` environment variable or pass `search_engine_id` parameter."
    );
}

#[rstest]
#[case::caller_engine_wins(Some("caller-engine"), Some("env-engine"), "caller-engine")]
#[case::env_engine_used(Some(""), Some("env-engine"), "env-engine")]
#[case::caller_engine_without_env(Some("caller-engine"), None, "caller-engine")]
fn engine_id_comes_from_the_caller_then_the_environment(
    #[case] caller_engine: Option<&str>,
    #[case] env_engine: Option<&str>,
    #[case] expected: &str,
) {
    let mut entries = Vec::new();
    if let Some(value) = env_engine {
        entries.push(("GOOGLE_PSE_ENGINE_ID", value));
    }
    let lookup = environment(&entries);
    let mut search = provider(&lookup);
    search.search_engine_id = caller_engine.map(ToOwned::to_owned);

    let body = search
        .transform_search_request(&single("rust"), &Object::new())
        .unwrap();

    assert_eq!(
        body.params.get("cx"),
        Some(&SearchParam::Text(expected.to_owned()))
    );
}

#[test]
fn transform_reports_the_second_missing_key_text() {
    let lookup = environment(&[]);
    let mut search = provider(&lookup);
    search.api_key = None;

    let error = search
        .transform_search_request(&single("rust"), &Object::new())
        .unwrap_err();

    assert_eq!(error.to_string(), "GOOGLE_PSE_API_KEY is required");
}

#[test]
fn transform_reports_the_second_missing_engine_id_text() {
    let lookup = environment(&[]);
    let mut search = provider(&lookup);
    search.search_engine_id = None;

    let error = search
        .transform_search_request(&single("rust"), &Object::new())
        .unwrap_err();

    assert_eq!(error.to_string(), "GOOGLE_PSE_ENGINE_ID is required");
}

#[test]
fn transform_keeps_the_source_record_order_in_the_query_string() {
    let lookup = environment(&[]);
    let options = params(
        r#"{"max_results": 15, "search_domain_filter": ["first.example", "second.example"], "country": "us", "start": 3}"#,
    );

    let body = provider(&lookup)
        .transform_search_request(&single("rust search"), &options)
        .unwrap();
    let url = provider(&lookup).complete_url(None, Some(&body)).unwrap();

    assert_eq!(
        url,
        format!(
            "{DEFAULT_BASE}?q=rust+search&cx=engine-id&key=caller-key&num=10&siteSearch=first.example&siteSearchFilter=i&gl=US&start=3"
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

#[rstest]
#[case::only_first_domain(r#"["first.example", "second.example"]"#, Some("first.example"))]
#[case::string_domain(r#""only.example""#, Some("only.example"))]
#[case::empty_list("[]", None)]
#[case::number_ignored("7", None)]
fn site_search_takes_the_first_domain_or_the_string(
    #[case] filter: &str,
    #[case] expected: Option<&str>,
) {
    let lookup = environment(&[]);
    let options = params(&format!(r#"{{"search_domain_filter": {filter}}}"#));

    let body = provider(&lookup)
        .transform_search_request(&single("rust"), &options)
        .unwrap();

    assert_eq!(
        body.params.get("siteSearch"),
        expected
            .map(|domain| SearchParam::Text(domain.to_owned()))
            .as_ref()
    );
    assert_eq!(
        body.params.get("siteSearchFilter").is_some(),
        expected.is_some()
    );
}

#[test]
fn country_is_uppercased_into_gl() {
    let lookup = environment(&[]);
    let options = params(r#"{"country": "de"}"#);

    let body = provider(&lookup)
        .transform_search_request(&single("rust"), &options)
        .unwrap();

    assert_eq!(
        body.params.get("gl"),
        Some(&SearchParam::Text(String::from("DE")))
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
    let options = params(r#"{"cx": "caller-cx", "key": "caller-key-2", "q": "other"}"#);

    let body = provider(&lookup)
        .transform_search_request(&single("rust"), &options)
        .unwrap();

    assert_eq!(
        body.params.get("cx"),
        Some(&SearchParam::Text(String::from("engine-id")))
    );
    assert_eq!(
        body.params.get("q"),
        Some(&SearchParam::Text(String::from("rust")))
    );
    assert_eq!(
        body.params.get("key"),
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
        format!("{DEFAULT_BASE}?{expected}&cx=engine-id&key=caller-key")
    );
}

#[rstest]
#[case::caller_base(Some("https://proxy.test/v1"), &[("GOOGLE_PSE_API_BASE", "https://env.test")], "https://proxy.test/v1")]
#[case::environment_base(None, &[("GOOGLE_PSE_API_BASE", "https://env.test")], "https://env.test")]
#[case::empty_caller_base(None, &[("GOOGLE_PSE_API_BASE", "")], DEFAULT_BASE)]
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
    let lookup = environment(&[("GOOGLE_PSE_API_KEY", "server-key")]);
    let mut search = provider(&lookup);
    search.api_key = None;
    search.api_base = Some(String::from("https://evil.example/v1"));

    let error = search.validate_environment(Headers::new()).unwrap_err();

    assert!(matches!(error, GooglePseError::Credential(_)));
    assert!(!error.to_string().contains("server-key"));
}

#[test]
fn the_second_resolution_repeats_the_host_check() {
    let lookup = environment(&[("GOOGLE_PSE_API_KEY", "server-key")]);
    let mut search = provider(&lookup);
    search.api_key = None;
    search.api_base = Some(String::from("https://evil.example/v1"));

    let error = search
        .transform_search_request(&single("rust"), &Object::new())
        .unwrap_err();

    assert!(matches!(error, GooglePseError::Credential(_)));
    assert!(!error.to_string().contains("server-key"));
}

#[rstest]
#[case::default_host("https://www.googleapis.com/other/path", &[])]
#[case::operator_override("https://proxy.internal:9000/google", &[("GOOGLE_PSE_API_BASE", "https://proxy.internal:9000")])]
fn an_environment_key_reaches_a_trusted_host(
    #[case] api_base: &str,
    #[case] entries: &[(&str, &str)],
) {
    let mut all = vec![("GOOGLE_PSE_API_KEY", "server-key")];
    all.extend_from_slice(entries);
    let lookup = environment(&all);
    let mut search = provider(&lookup);
    search.api_key = None;
    search.api_base = Some(api_base.to_owned());

    let body = search
        .transform_search_request(&single("rust"), &Object::new())
        .unwrap();

    assert_eq!(
        body.params.get("key"),
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
        body.params.get("key"),
        Some(&SearchParam::Text(String::from("caller-key")))
    );
}

#[test]
fn a_reply_without_items_gives_no_results() {
    let response = decode_reply(r#"{"searchInformation": {"totalResults": "0"}}"#).unwrap();

    assert!(response.results.is_empty());
    assert_eq!(response.object.0, "search");
}

#[test]
fn a_reply_item_projects_title_link_and_snippet() {
    let response = decode_reply(
        r#"{"items": [{"title": "Rust", "link": "https://rust.example", "snippet": "A language.", "pagemap": {}}]}"#,
    )
    .unwrap();

    let result = response.results.first().unwrap();
    assert_eq!(result.title, "Rust");
    assert_eq!(result.url, "https://rust.example");
    assert_eq!(result.snippet, "A language.");
    assert_eq!(result.date, None);
    assert_eq!(result.last_updated, None);
}

#[test]
fn a_reply_item_missing_a_field_defaults_to_empty() {
    let response = decode_reply(r#"{"items": [{"title": "Rust"}]}"#).unwrap();

    let result = response.results.first().unwrap();
    assert_eq!(result.title, "Rust");
    assert_eq!(result.url, "");
    assert_eq!(result.snippet, "");
}

#[rstest]
#[case::null_title(r#"{"items": [{"title": null, "link": "l", "snippet": "s"}]}"#)]
#[case::items_null(r#"{"items": null}"#)]
#[case::not_json("not json")]
#[case::items_not_a_list(r#"{"items": {}}"#)]
fn a_malformed_reply_is_an_error(#[case] text: &str) {
    drop(decode_reply(text).unwrap_err());
}

#[rstest]
fn replays_the_google_pse_corpus(#[files("fuzz/corpus/google_pse_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    drop(decode_reply(text));
}

#[test]
fn the_secret_string_in_a_request_is_the_resolved_key() {
    let lookup = environment(&[("GOOGLE_PSE_API_KEY", "server-key")]);
    let mut search = provider(&lookup);
    search.api_key = Some(SecretString::from("caller-key"));

    let body = search
        .transform_search_request(&single("rust"), &Object::new())
        .unwrap();

    let Some(SearchParam::Text(sent)) = body.params.get("key") else {
        panic!("the record keeps the key as text");
    };
    assert_eq!(sent, "caller-key");
    assert_eq!(
        search.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("caller-key")
    );
}

#[test]
fn debug_never_shows_the_key() {
    let lookup = environment(&[]);

    let rendered = format!("{:?}", provider(&lookup));

    assert!(!rendered.contains("caller-key"));
    assert!(rendered.contains("GooglePseSearch"));
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
