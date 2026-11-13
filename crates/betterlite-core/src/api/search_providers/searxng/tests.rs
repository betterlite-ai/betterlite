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
use secrecy::SecretString;

use super::{
    SearxngSearchConfig, SearxngSearchError, build_request, decode_searxng_reply, search_url,
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

fn config(variables: &[(&str, &str)]) -> SearxngSearchConfig {
    SearxngSearchConfig::load(None, None, &environment(variables)).unwrap()
}

fn headers(config: &SearxngSearchConfig, caller: &[(&str, &str)]) -> BTreeMap<String, String> {
    let sent: BTreeMap<String, String> = caller
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    config.validate_environment(sent, None, None)
}

#[test]
fn searxng_search_settings_work_without_a_key() {
    let loaded = config(&[]);
    let sent = headers(&loaded, &[("x-trace", "t-1")]);
    assert_eq!(sent.get("Authorization"), None);
    assert_eq!(
        sent.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(sent.get("x-trace").map(String::as_str), Some("t-1"));
}

#[test]
fn searxng_search_settings_send_the_environment_key() {
    let loaded = config(&[("SEARXNG_API_KEY", "sk-searxng")]);
    assert_eq!(
        headers(&loaded, &[])
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer sk-searxng")
    );
}

#[test]
fn searxng_search_settings_prefer_the_caller_key() {
    let loaded = config(&[("SEARXNG_API_KEY", "sk-server")]);
    let caller = SecretString::from("sk-caller");
    let sent = loaded.validate_environment(BTreeMap::new(), Some(&caller), None);
    assert_eq!(
        sent.get("Authorization").map(String::as_str),
        Some("Bearer sk-caller")
    );
}

#[test]
fn searxng_search_settings_treat_an_empty_key_as_unset() {
    let loaded = config(&[("SEARXNG_API_KEY", "")]);
    assert_eq!(headers(&loaded, &[]).get("Authorization"), None);
}

#[test]
fn searxng_search_settings_treat_an_empty_base_as_unset() {
    let loaded = config(&[("SEARXNG_API_BASE", "")]);
    assert!(matches!(
        loaded.complete_url(None, &Object::new(), None),
        Err(SearxngSearchError::MissingApiBase)
    ));
}

#[test]
fn searxng_search_settings_refuse_a_caller_base_with_a_server_key() {
    let lookup = environment(&[("SEARXNG_API_KEY", "sk-server")]);
    let error = SearxngSearchConfig::load(None, Some("https://searx.test"), &lookup).unwrap_err();
    assert!(matches!(
        error,
        SearxngSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
}

#[test]
fn searxng_search_settings_trust_the_environment_base() {
    let lookup = environment(&[
        ("SEARXNG_API_KEY", "sk-server"),
        ("SEARXNG_API_BASE", "https://searx.test"),
    ]);
    let loaded = SearxngSearchConfig::load(None, Some("https://searx.test/"), &lookup).unwrap();
    assert_eq!(
        loaded
            .complete_url(Some("https://searx.test/"), &Object::new(), None)
            .unwrap(),
        "https://searx.test/search"
    );
}

#[test]
fn searxng_search_settings_let_a_caller_key_bypass_the_trust_check() {
    let caller = SecretString::from("sk-caller");
    let loaded =
        SearxngSearchConfig::load(Some(caller), Some("https://searx.test"), &environment(&[]))
            .unwrap();
    assert_eq!(
        loaded
            .validate_environment(BTreeMap::new(), None, None)
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer sk-caller")
    );
}

#[test]
fn searxng_request_builds_the_params_record() {
    let request = build_request(&SearchQuery::Single("rust".to_owned()), &Object::new()).unwrap();
    assert_eq!(request.params.query, "rust");
    assert_eq!(request.params.format, "json");
    assert_eq!(request.params.language, None);
    assert!(request.params.extra.is_empty());
}

#[test]
fn searxng_request_joins_a_query_list() {
    let query = SearchQuery::Multiple(vec!["latest".to_owned(), "rust".to_owned()]);
    let request = build_request(&query, &Object::new()).unwrap();
    assert_eq!(request.params.query, "latest rust");
}

#[rstest]
#[case::us("us", "en")]
#[case::uk("uk", "en")]
#[case::de("de", "de")]
#[case::fr("fr", "fr")]
#[case::es("es", "es")]
#[case::jp("jp", "ja")]
#[case::other("br", "br")]
#[case::uppercase("US", "en")]
#[case::empty("", "")]
fn searxng_request_maps_country_to_language(#[case] country: &str, #[case] language: &str) {
    let options = params(&[("country", text(country))]);
    let request = build_request(&SearchQuery::Single("q".to_owned()), &options).unwrap();
    assert_eq!(request.params.language.as_deref(), Some(language));
}

#[test]
fn searxng_request_defaults_a_null_country() {
    let options = params(&[("country", JsonValue::Null)]);
    let request = build_request(&SearchQuery::Single("q".to_owned()), &options).unwrap();
    assert_eq!(request.params.language, None);
}

#[test]
fn searxng_request_drops_and_passes_options() {
    let options = params(&[
        ("max_results", JsonValue::from(5_i64)),
        (
            "search_domain_filter",
            JsonValue::Array(vec![text("a.test")]),
        ),
        ("max_tokens_per_page", JsonValue::from(100_i64)),
        ("categories", text("news")),
        ("q", text("ignored")),
        ("format", text("csv")),
        ("language", text("fr")),
    ]);
    let request = build_request(&SearchQuery::Single("q".to_owned()), &options).unwrap();
    assert_eq!(request.params.query, "q");
    assert_eq!(request.params.format, "json");
    assert_eq!(request.params.language, None);
    assert_eq!(request.params.extra.len(), 2);
    assert_eq!(request.params.extra.get("categories"), Some(&text("news")));
    assert_eq!(request.params.extra.get("language"), Some(&text("fr")));
}

#[test]
fn searxng_request_keeps_the_mapped_language_over_a_caller_language() {
    let options = params(&[("country", text("us")), ("language", text("fr"))]);
    let request = build_request(&SearchQuery::Single("q".to_owned()), &options).unwrap();
    assert_eq!(request.params.language.as_deref(), Some("en"));
    assert_eq!(request.params.extra.get("language"), None);
    let url = search_url(Some("https://searx.test"), None, Some(&request)).unwrap();
    let languages: Vec<&str> = url
        .split_once('?')
        .map(|(_, query)| {
            query
                .split('&')
                .filter(|pair| pair.starts_with("language="))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(languages, ["language=en"]);
}

#[test]
fn searxng_request_error_names_the_option() {
    let options = params(&[("country", JsonValue::from(5_i64))]);
    let error = build_request(&SearchQuery::Single("q".to_owned()), &options).unwrap_err();
    assert!(matches!(
        error,
        SearxngSearchError::OptionType { field: "country" }
    ));
}

#[rstest]
#[case::plain("https://searx.test", "https://searx.test/search")]
#[case::trailing_slash("https://searx.test/", "https://searx.test/search")]
#[case::existing_suffix("https://searx.test/search", "https://searx.test/search")]
fn searxng_url_appends_the_search_suffix(#[case] base: &str, #[case] expected: &str) {
    let request = build_request(&SearchQuery::Single("q".to_owned()), &Object::new()).unwrap();
    assert_eq!(
        search_url(Some(base), None, Some(&request)).unwrap(),
        format!("{expected}?q=q&format=json")
    );
}

#[test]
fn searxng_url_uses_the_environment_base() {
    assert_eq!(
        search_url(None, Some("https://searx.test/"), None).unwrap(),
        "https://searx.test/search"
    );
}

#[test]
fn searxng_url_requires_a_base() {
    assert!(matches!(
        search_url(None, None, None),
        Err(SearxngSearchError::MissingApiBase)
    ));
}

#[test]
fn searxng_url_encodes_the_params_record() {
    let options = params(&[
        ("country", text("us")),
        ("mark", text("~*")),
        ("page", JsonValue::from(2_i64)),
        ("safe", JsonValue::Bool(true)),
        ("tags", JsonValue::Array(vec![text("a"), text("b")])),
    ]);
    let query = SearchQuery::Multiple(vec!["rust".to_owned(), "lang".to_owned()]);
    let request = build_request(&query, &options).unwrap();
    let url = search_url(Some("https://searx.test"), None, Some(&request)).unwrap();
    assert_eq!(
        url,
        "https://searx.test/search?q=rust+lang&format=json&language=en&mark=~%2A&page=2&safe=True&tags=%5B%27a%27%2C+%27b%27%5D"
    );
}

#[test]
fn searxng_url_quotes_wire_text_like_python_repr() {
    let options = params(&[(
        "tags",
        JsonValue::Array(vec![text("O'Brien"), text("a\"b'c")]),
    )]);
    let request = build_request(&SearchQuery::Single("q".to_owned()), &options).unwrap();
    let url = search_url(Some("https://searx.test"), None, Some(&request)).unwrap();
    assert_eq!(
        url,
        "https://searx.test/search?q=q&format=json&tags=%5B%22O%27Brien%22%2C+%27a%22b%5C%27c%27%5D"
    );
}

#[test]
fn searxng_url_without_a_record_has_no_query() {
    assert_eq!(
        search_url(Some("https://searx.test"), None, None).unwrap(),
        "https://searx.test/search"
    );
}

#[test]
fn searxng_reply_maps_the_pinned_fields() {
    let body = br#"{"results":[{"title":"Rust","url":"https://rust.test","content":"safe","publishedDate":"2024-01-02"}]}"#;
    let response = decode_searxng_reply(body).unwrap();
    assert_eq!(response.object.0, "search");
    let result = &response.results[0];
    assert_eq!(
        (
            result.title.as_str(),
            result.url.as_str(),
            result.snippet.as_str()
        ),
        ("Rust", "https://rust.test", "safe")
    );
    assert_eq!(result.date.as_deref(), Some("2024-01-02"));
    assert_eq!(result.last_updated, None);
}

#[rstest]
#[case::pubdate_only(
    br#"{"results":[{"title":"T","url":"u","content":"c","pubdate":"2023-05-06"}]}"#,
    Some("2023-05-06")
)]
#[case::published_date_wins(br#"{"results":[{"title":"T","url":"u","content":"c","publishedDate":"2024-01-02","pubdate":"2023-05-06"}]}"#, Some("2024-01-02"))]
#[case::null_published_date(br#"{"results":[{"title":"T","url":"u","content":"c","publishedDate":null,"pubdate":"2023-05-06"}]}"#, Some("2023-05-06"))]
#[case::no_date(br#"{"results":[{"title":"T","url":"u","content":"c"}]}"#, None)]
fn searxng_reply_reads_the_date(#[case] body: &[u8], #[case] expected: Option<&str>) {
    let response = decode_searxng_reply(body).unwrap();
    assert_eq!(response.results[0].date.as_deref(), expected);
}

#[test]
fn searxng_reply_defaults_missing_fields() {
    let body = br#"{"results":[{}]}"#;
    let response = decode_searxng_reply(body).unwrap();
    assert_eq!(
        (
            response.results[0].title.as_str(),
            response.results[0].url.as_str(),
            response.results[0].snippet.as_str()
        ),
        ("", "", "")
    );
}

#[rstest]
#[case::no_results(br#"{"query":"q"}"#)]
#[case::empty_results(br#"{"results":[]}"#)]
fn searxng_reply_without_results_is_empty(#[case] body: &[u8]) {
    let response = decode_searxng_reply(body).unwrap();
    assert!(response.results.is_empty());
}

#[test]
fn searxng_reply_rejects_malformed_json() {
    assert!(matches!(
        decode_searxng_reply(b"{"),
        Err(SearxngSearchError::ReplyJson { .. })
    ));
}

#[test]
fn searxng_config_reports_its_display_name_and_method() {
    assert_eq!(SearxngSearchConfig::ui_friendly_name(), "SearXNG");
    let loaded = config(&[]);
    assert_eq!(loaded.http_method(), HttpMethod::Get);
    assert!(!loaded.supports_rich_search_input());
}

#[test]
fn searxng_response_hook_decodes_the_provider_reply() {
    let loaded = config(&[]);
    let headers = BTreeMap::new();
    let response = ProviderResponse {
        status_code: 200,
        headers: &headers,
        text: r#"{"results":[{"title":"Rust","url":"https://rust.test"}]}"#,
    };
    let decoded = loaded.transform_search_response(&response).unwrap();
    assert_eq!(decoded.results[0].title, "Rust");
}

#[rstest]
fn replays_the_searxng_corpus(#[files("fuzz/corpus/searxng_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(response) = decode_searxng_reply(&bytes) {
        assert_eq!(str::from_utf8(&bytes).map(str::len), Ok(bytes.len()));
        drop(response.results);
    }
}
