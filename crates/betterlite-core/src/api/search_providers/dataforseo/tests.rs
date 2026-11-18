#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::CredentialError;
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, SearchProvider as _, SearchQuery,
};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::reply::{REPLY_MAX_BYTES, decode_dataforseo_search};
use super::{
    DATAFORSEO_API_BASE, DataForSeoSearchConfig, DataForSeoSearchError, DataForSeoSearchSettings,
};

fn environment(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn config(values: &[(&str, &str)]) -> DataForSeoSearchConfig {
    DataForSeoSearchConfig {
        settings: DataForSeoSearchSettings::load(&environment(values)),
    }
}

fn pair() -> DataForSeoSearchConfig {
    config(&[
        ("DATAFORSEO_LOGIN", "user"),
        ("DATAFORSEO_PASSWORD", "secret"),
    ])
}

fn basic(credentials: &str) -> String {
    format!("Basic {}", STANDARD.encode(credentials))
}

fn error_of(
    config: &DataForSeoSearchConfig,
    api_key: Option<&SecretString>,
) -> DataForSeoSearchError {
    config
        .validated_headers(Headers::new(), api_key, Some("https://evil.test"))
        .unwrap_err()
}

#[test]
fn dataforseo_reports_its_display_name_and_post_method() {
    assert_eq!(DataForSeoSearchConfig::ui_friendly_name(), "DataForSEO");
    assert_eq!(config(&[]).http_method(), HttpMethod::Post);
}

#[rstest]
#[case::unset(&[], None, DATAFORSEO_API_BASE, 2250)]
#[case::both(
    &[("DATAFORSEO_LOGIN", "user"), ("DATAFORSEO_PASSWORD", "secret")],
    Some("user"),
    DATAFORSEO_API_BASE,
    2250
)]
#[case::empty(
    &[("DATAFORSEO_LOGIN", ""), ("DATAFORSEO_PASSWORD", "")],
    None,
    DATAFORSEO_API_BASE,
    2250
)]
#[case::base(
    &[("DATAFORSEO_API_BASE", "https://proxy.test/v3")],
    None,
    "https://proxy.test/v3",
    2250
)]
#[case::empty_base(&[("DATAFORSEO_API_BASE", "")], None, DATAFORSEO_API_BASE, 2250)]
#[case::location(
    &[("DEFAULT_DATAFORSEO_LOCATION_CODE", "2840")],
    None,
    DATAFORSEO_API_BASE,
    2840
)]
#[case::unparsable_location(
    &[("DEFAULT_DATAFORSEO_LOCATION_CODE", "usa")],
    None,
    DATAFORSEO_API_BASE,
    2250
)]
fn dataforseo_search_settings_load_login_password_base_and_location(
    #[case] values: &[(&str, &str)],
    #[case] expected_login: Option<&str>,
    #[case] expected_base: &str,
    #[case] expected_location: i64,
) {
    let settings = DataForSeoSearchSettings::load(&environment(values));
    assert_eq!(
        settings.login.as_ref().map(ExposeSecret::expose_secret),
        expected_login
    );
    assert_eq!(settings.api_base, expected_base);
    assert_eq!(settings.location_code, expected_location);
}

#[test]
fn dataforseo_search_settings_load_the_password() {
    let settings = DataForSeoSearchSettings::load(&environment(&[
        ("DATAFORSEO_LOGIN", "user"),
        ("DATAFORSEO_PASSWORD", "secret"),
    ]));
    assert_eq!(
        settings.password.as_ref().map(ExposeSecret::expose_secret),
        Some("secret")
    );
}

#[test]
fn dataforseo_settings_answer_the_guard_only_for_its_own_names() {
    let settings = pair().settings;
    assert_eq!(settings.lookup("DATAFORSEO_LOGIN").as_deref(), Some("user"));
    assert_eq!(
        settings.lookup("DATAFORSEO_API_BASE").as_deref(),
        Some(DATAFORSEO_API_BASE)
    );
    assert_eq!(settings.lookup("DATAFORSEO_PASSWORD"), None);
    let login_only = config(&[("DATAFORSEO_LOGIN", "user")]).settings;
    assert_eq!(login_only.lookup("DATAFORSEO_LOGIN"), None);
}

#[test]
fn dataforseo_headers_encode_the_environment_pair() {
    let mut headers = Headers::new();
    headers.insert("A".to_owned(), "b".to_owned());
    let validated = pair()
        .validated_headers(headers, None, Some("https://api.dataforseo.com/v3"))
        .unwrap();
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some(basic("user:secret").as_str())
    );
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(validated.get("A").map(String::as_str), Some("b"));
}

#[test]
fn dataforseo_caller_pair_overrides_the_environment_and_its_host() {
    let key = SecretString::from("caller:key");
    let validated = pair()
        .validated_headers(Headers::new(), Some(&key), Some("https://evil.test"))
        .unwrap();
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some(basic("caller:key").as_str())
    );
}

#[test]
fn dataforseo_caller_pair_splits_once_on_the_first_colon() {
    let key = SecretString::from("login:pass:word");
    let validated = config(&[])
        .validated_headers(Headers::new(), Some(&key), None)
        .unwrap();
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some(basic("login:pass:word").as_str())
    );
}

#[rstest]
#[case::both_empty(":")]
#[case::empty_login(":secret")]
fn dataforseo_caller_pair_needs_a_login(#[case] key: &str) {
    let secret = SecretString::from(key);
    let error = config(&[])
        .validated_headers(Headers::new(), Some(&secret), None)
        .unwrap_err();
    assert!(matches!(error, DataForSeoSearchError::MissingLogin));
}

#[test]
fn dataforseo_caller_pair_needs_a_password() {
    let secret = SecretString::from("user:");
    let error = config(&[])
        .validated_headers(Headers::new(), Some(&secret), None)
        .unwrap_err();
    assert!(matches!(error, DataForSeoSearchError::MissingPassword));
}

#[test]
fn dataforseo_noncolon_key_is_ignored() {
    let key = SecretString::from("sk-caller");
    let validated = pair()
        .validated_headers(Headers::new(), Some(&key), None)
        .unwrap();
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some(basic("user:secret").as_str())
    );
    let error = config(&[])
        .validated_headers(Headers::new(), Some(&key), None)
        .unwrap_err();
    assert!(matches!(error, DataForSeoSearchError::MissingLogin));
}

#[test]
fn dataforseo_noncolon_key_does_not_bypass_the_base_guard() {
    let key = SecretString::from("sk-caller");
    let error = error_of(&pair(), Some(&key));
    assert!(matches!(
        &error,
        DataForSeoSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
    assert!(!error.to_string().contains("secret"));
}

#[rstest]
#[case::default_host(None, true)]
#[case::default_endpoint(
    Some("https://api.dataforseo.com/v3/serp/google/organic/live/advanced"),
    true
)]
#[case::other_host(Some("https://evil.test"), false)]
#[case::other_port(Some("https://api.dataforseo.com:8443"), false)]
#[case::empty_base(Some(""), true)]
#[case::env_host(Some("https://proxy.test/v3"), true)]
fn dataforseo_environment_pair_needs_a_trusted_base(
    #[case] api_base: Option<&str>,
    #[case] trusted: bool,
) {
    let config = config(&[
        ("DATAFORSEO_LOGIN", "user"),
        ("DATAFORSEO_PASSWORD", "secret"),
        ("DATAFORSEO_API_BASE", "https://proxy.test/v3"),
    ]);
    let validated = config.validated_headers(Headers::new(), None, api_base);
    assert_eq!(validated.is_ok(), trusted, "{validated:?}");
}

#[test]
fn dataforseo_untrusted_base_keeps_the_credentials_out_of_the_error() {
    let error = error_of(&pair(), None);
    assert!(matches!(
        &error,
        DataForSeoSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
    assert!(error.to_string().contains("DATAFORSEO_LOGIN"));
    assert!(!error.to_string().contains("secret"));
    assert!(!error.to_string().contains("user"));
}

#[test]
fn dataforseo_missing_login_keeps_the_source_message() {
    let error = config(&[])
        .validated_headers(Headers::new(), None, None)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "DATAFORSEO_LOGIN is not set. Set `DATAFORSEO_LOGIN` environment variable or pass credentials in api_key parameter."
    );
}

#[test]
fn dataforseo_missing_password_keeps_the_source_message() {
    let error = config(&[("DATAFORSEO_LOGIN", "user")])
        .validated_headers(Headers::new(), None, None)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "DATAFORSEO_PASSWORD is not set. Set `DATAFORSEO_PASSWORD` environment variable or pass credentials in api_key parameter."
    );
}

#[test]
fn dataforseo_missing_login_is_reported_before_a_missing_password() {
    let error = config(&[("DATAFORSEO_PASSWORD", "secret")])
        .validated_headers(Headers::new(), None, None)
        .unwrap_err();
    assert!(matches!(error, DataForSeoSearchError::MissingLogin));
}

#[test]
fn dataforseo_partial_environment_pair_skips_the_base_guard() {
    let error = config(&[("DATAFORSEO_LOGIN", "user")])
        .validated_headers(Headers::new(), None, Some("https://evil.test"))
        .unwrap_err();
    assert!(matches!(error, DataForSeoSearchError::MissingPassword));
}

#[test]
fn dataforseo_validate_environment_sets_the_basic_header() {
    let validated = pair().validate_environment(Headers::new(), None, None);
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some(basic("user:secret").as_str())
    );
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn dataforseo_validate_environment_leaves_headers_alone_on_a_refusal() {
    let mut headers = Headers::new();
    headers.insert("A".to_owned(), "b".to_owned());
    let kept = config(&[]).validate_environment(headers.clone(), None, None);
    assert_eq!(kept, headers);
}

#[test]
fn dataforseo_sign_request_signs_with_the_server_pair() {
    let config = pair();
    let text = SearchQuery::Single(String::from("rust"));
    let request = config
        .transform_search_request(&text, &Object::new())
        .unwrap();
    let outbound = OutboundSearch {
        api_base: DATAFORSEO_API_BASE,
        api_key: None,
        optional_params: &Object::new(),
        body: &request,
    };
    let (headers, body) = config.sign_request(Headers::new(), &outbound).unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some(basic("user:secret").as_str())
    );
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(body, None);
}

#[test]
fn dataforseo_sign_request_fails_before_sending_without_credentials() {
    let config = config(&[]);
    let text = SearchQuery::Single(String::from("rust"));
    let request = config
        .transform_search_request(&text, &Object::new())
        .unwrap();
    let outbound = OutboundSearch {
        api_base: DATAFORSEO_API_BASE,
        api_key: None,
        optional_params: &Object::new(),
        body: &request,
    };
    let error = config.sign_request(Headers::new(), &outbound).unwrap_err();
    assert!(matches!(error, DataForSeoSearchError::MissingLogin));
}

#[test]
fn dataforseo_sign_request_blocks_the_server_pair_for_an_untrusted_base() {
    let config = pair();
    let text = SearchQuery::Single(String::from("rust"));
    let request = config
        .transform_search_request(&text, &Object::new())
        .unwrap();
    let outbound = OutboundSearch {
        api_base: "https://evil.test/v3",
        api_key: None,
        optional_params: &Object::new(),
        body: &request,
    };
    let error = config.sign_request(Headers::new(), &outbound).unwrap_err();
    assert!(matches!(
        error,
        DataForSeoSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
}

#[rstest]
#[case::default(None, &[], DATAFORSEO_API_BASE)]
#[case::caller(Some("https://proxy.test/v3"), &[], "https://proxy.test/v3")]
#[case::trailing_slash(Some("https://proxy.test/v3/"), &[], "https://proxy.test/v3/")]
#[case::empty_caller(Some(""), &[], DATAFORSEO_API_BASE)]
#[case::env(None, &[("DATAFORSEO_API_BASE", "https://env.test/v3")], "https://env.test/v3")]
#[case::empty_env(None, &[("DATAFORSEO_API_BASE", "")], DATAFORSEO_API_BASE)]
fn dataforseo_url_is_the_resolved_endpoint_unchanged(
    #[case] api_base: Option<&str>,
    #[case] values: &[(&str, &str)],
    #[case] expected: &str,
) {
    let url = config(values)
        .complete_url(api_base, &Object::new(), None)
        .unwrap();
    assert_eq!(url, expected);
}

#[test]
fn dataforseo_url_ignores_the_parameters_and_the_body() {
    let config = config(&[]);
    let text = SearchQuery::Single(String::from("rust"));
    let body = config
        .transform_search_request(&text, &Object::new())
        .unwrap();
    let options: Object = serde_json::from_str(r#"{"max_results": 5}"#).unwrap();
    let url = config
        .complete_url(Some("https://proxy.test/v3"), &options, Some(&body))
        .unwrap();
    assert_eq!(url, "https://proxy.test/v3");
}

fn request_tasks(text: &str, options: &str) -> Vec<IndexMap<String, Json>> {
    let optional_params: Object = serde_json::from_str(options).unwrap();
    let query: SearchQuery = serde_json::from_str(text).unwrap();
    let body = config(&[])
        .transform_search_request(&query, &optional_params)
        .unwrap();
    serde_json::from_str(&serde_json::to_string(&body).unwrap()).unwrap()
}

fn expected_tasks(text: &str) -> Vec<IndexMap<String, Json>> {
    serde_json::from_str(text).unwrap()
}

fn request_error(options: &str) -> DataForSeoSearchError {
    let optional_params: Object = serde_json::from_str(options).unwrap();
    let query = SearchQuery::Single(String::from("q"));
    config(&[])
        .transform_search_request(&query, &optional_params)
        .unwrap_err()
}

#[test]
fn dataforseo_request_is_a_one_element_task_array() {
    let options = r#"{"max_results": 5, "country": "France", "search_domain_filter": ["a.com"], "device": "mobile"}"#;
    let optional_params: Object = serde_json::from_str(options).unwrap();
    let query = SearchQuery::Single(String::from("q"));
    let body = config(&[])
        .transform_search_request(&query, &optional_params)
        .unwrap();
    assert_eq!(body.0.len(), 1);
}

#[rstest]
#[case::plain(
    "\"q\"",
    "{}",
    r#"[{"keyword": "q", "language_code": "en", "location_code": 2250}]"#
)]
#[case::first_of_list(
    "[\"a\", \"b\"]",
    "{}",
    r#"[{"keyword": "a", "language_code": "en", "location_code": 2250}]"#
)]
#[case::empty_list(
    "[]",
    "{}",
    r#"[{"keyword": "", "language_code": "en", "location_code": 2250}]"#
)]
#[case::depth(
    "\"q\"",
    r#"{"max_results": 5}"#,
    r#"[{"keyword": "q", "depth": 5, "language_code": "en", "location_code": 2250}]"#
)]
#[case::zero_depth(
    "\"q\"",
    r#"{"max_results": 0}"#,
    r#"[{"keyword": "q", "language_code": "en", "location_code": 2250}]"#
)]
#[case::null_depth(
    "\"q\"",
    r#"{"max_results": null}"#,
    r#"[{"keyword": "q", "language_code": "en", "location_code": 2250}]"#
)]
#[case::negative_depth(
    "\"q\"",
    r#"{"max_results": -3}"#,
    r#"[{"keyword": "q", "depth": -3, "language_code": "en", "location_code": 2250}]"#
)]
#[case::depth_at_cap(
    "\"q\"",
    r#"{"max_results": 700}"#,
    r#"[{"keyword": "q", "depth": 700, "language_code": "en", "location_code": 2250}]"#
)]
#[case::depth_over_cap(
    "\"q\"",
    r#"{"max_results": 701}"#,
    r#"[{"keyword": "q", "depth": 700, "language_code": "en", "location_code": 2250}]"#
)]
#[case::country(
    "\"q\"",
    r#"{"country": "France"}"#,
    r#"[{"keyword": "q", "location_name": "France", "language_code": "en"}]"#
)]
#[case::empty_country(
    "\"q\"",
    r#"{"country": ""}"#,
    r#"[{"keyword": "q", "language_code": "en", "location_code": 2250}]"#
)]
#[case::null_country(
    "\"q\"",
    r#"{"country": null}"#,
    r#"[{"keyword": "q", "language_code": "en", "location_code": 2250}]"#
)]
#[case::domains(
    "\"q\"",
    r#"{"search_domain_filter": ["a.com", "b.com"]}"#,
    r#"[{"keyword": "q", "domain": ["a.com", "b.com"], "language_code": "en", "location_code": 2250}]"#
)]
#[case::empty_domains(
    "\"q\"",
    r#"{"search_domain_filter": []}"#,
    r#"[{"keyword": "q", "language_code": "en", "location_code": 2250}]"#
)]
#[case::null_domains(
    "\"q\"",
    r#"{"search_domain_filter": null}"#,
    r#"[{"keyword": "q", "language_code": "en", "location_code": 2250}]"#
)]
#[case::caller_language_code(
    "\"q\"",
    r#"{"language_code": "fr"}"#,
    r#"[{"keyword": "q", "language_code": "en", "location_code": 2250}]"#
)]
#[case::language_name_coexists(
    "\"q\"",
    r#"{"language_name": "French"}"#,
    r#"[{"keyword": "q", "language_code": "en", "location_code": 2250, "language_name": "French"}]"#
)]
#[case::caller_location_code(
    "\"q\"",
    r#"{"location_code": 2840}"#,
    r#"[{"keyword": "q", "language_code": "en", "location_code": 2250}]"#
)]
#[case::country_and_location_code(
    "\"q\"",
    r#"{"country": "France", "location_code": 2840}"#,
    r#"[{"keyword": "q", "location_name": "France", "language_code": "en", "location_code": 2840}]"#
)]
#[case::mapped_location_name_wins(
    "\"q\"",
    r#"{"country": "France", "location_name": "Germany"}"#,
    r#"[{"keyword": "q", "location_name": "France", "language_code": "en"}]"#
)]
#[case::caller_location_name(
    "\"q\"",
    r#"{"location_name": "France"}"#,
    r#"[{"keyword": "q", "language_code": "en", "location_code": 2250, "location_name": "France"}]"#
)]
#[case::caller_depth(
    "\"q\"",
    r#"{"depth": 3}"#,
    r#"[{"keyword": "q", "depth": 3, "language_code": "en", "location_code": 2250}]"#
)]
#[case::mapped_depth_wins(
    "\"q\"",
    r#"{"max_results": 5, "depth": 3}"#,
    r#"[{"keyword": "q", "depth": 5, "language_code": "en", "location_code": 2250}]"#
)]
#[case::zero_depth_keeps_the_native_one(
    "\"q\"",
    r#"{"max_results": 0, "depth": 9}"#,
    r#"[{"keyword": "q", "depth": 9, "language_code": "en", "location_code": 2250}]"#
)]
#[case::caller_domain(
    "\"q\"",
    r#"{"domain": "a.com"}"#,
    r#"[{"keyword": "q", "domain": "a.com", "language_code": "en", "location_code": 2250}]"#
)]
#[case::mapped_domain_wins(
    "\"q\"",
    r#"{"search_domain_filter": ["b.com"], "domain": "a.com"}"#,
    r#"[{"keyword": "q", "domain": ["b.com"], "language_code": "en", "location_code": 2250}]"#
)]
#[case::pagination_dropped(
    "\"q\"",
    r#"{"max_tokens_per_page": 500}"#,
    r#"[{"keyword": "q", "language_code": "en", "location_code": 2250}]"#
)]
#[case::native_passthrough(
    "\"q\"",
    r#"{"device": "mobile", "os": "android"}"#,
    r#"[{"keyword": "q", "language_code": "en", "location_code": 2250, "device": "mobile", "os": "android"}]"#
)]
#[case::keyword_passthrough_ignored(
    "\"q\"",
    r#"{"keyword": "x"}"#,
    r#"[{"keyword": "q", "language_code": "en", "location_code": 2250}]"#
)]
fn dataforseo_request_maps_the_unified_options(
    #[case] text: &str,
    #[case] options: &str,
    #[case] expected: &str,
) {
    assert_eq!(request_tasks(text, options), expected_tasks(expected));
}

#[rstest]
#[case::float_max_results(
    r#"{"max_results": 5.5}"#,
    "the DataForSEO search option `max_results` is not an integer"
)]
#[case::text_max_results(
    r#"{"max_results": "5"}"#,
    "the DataForSEO search option `max_results` is not an integer"
)]
#[case::numeric_country(
    r#"{"country": 5}"#,
    "the DataForSEO search option `country` is not a string"
)]
#[case::object_country(
    r#"{"country": {"a": 1}}"#,
    "the DataForSEO search option `country` is not a string"
)]
#[case::text_domains(
    r#"{"search_domain_filter": "a.com"}"#,
    "the DataForSEO search option `search_domain_filter` is not a list of strings"
)]
#[case::numeric_member(
    r#"{"search_domain_filter": [1]}"#,
    "the DataForSEO search option `search_domain_filter` is not a list of strings"
)]
fn dataforseo_request_rejects_a_wrong_option_type(#[case] options: &str, #[case] expected: &str) {
    assert_eq!(request_error(options).to_string(), expected);
}

fn reply_value(payload: &str) -> IndexMap<String, Json> {
    let response = decode_dataforse_search(payload.as_bytes()).unwrap();
    serde_json::from_str(&serde_json::to_string(&response).unwrap()).unwrap()
}

fn expected_reply(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::organic_only(
    r#"{"tasks": [{"status_code": 20000, "result": [{"items": [{"type": "organic", "title": "t", "url": "u", "description": "d"}, {"type": "news", "title": "n"}]}]}]}"#,
    r#"{"results": [{"title": "t", "url": "u", "snippet": "d", "date": null, "last_updated": null}], "object": "search"}"#
)]
#[case::missing_members(
    r#"{"tasks": [{"status_code": 20000, "result": [{"items": [{"type": "organic"}]}]}]}"#,
    r#"{"results": [{"title": "", "url": "", "snippet": "", "date": null, "last_updated": null}], "object": "search"}"#
)]
#[case::item_extras_dropped(
    r#"{"tasks": [{"status_code": 20000, "result": [{"items": [{"type": "organic", "title": "t", "position": 1, "domain": "a.com"}]}]}]}"#,
    r#"{"results": [{"title": "t", "url": "", "snippet": "", "date": null, "last_updated": null}], "object": "search"}"#
)]
#[case::item_dates_are_null(
    r#"{"tasks": [{"status_code": 20000, "result": [{"items": [{"type": "organic", "title": "t", "date": "2024-01-02", "last_updated": "2024-01-03"}]}]}]}"#,
    r#"{"results": [{"title": "t", "url": "", "snippet": "", "date": null, "last_updated": null}], "object": "search"}"#
)]
#[case::numeric_type_skipped(
    r#"{"tasks": [{"status_code": 20000, "result": [{"items": [{"type": 5, "title": "t"}]}]}]}"#,
    r#"{"results": [], "object": "search"}"#
)]
#[case::null_type_skipped(
    r#"{"tasks": [{"status_code": 20000, "result": [{"items": [{"type": null, "title": "t"}]}]}]}"#,
    r#"{"results": [], "object": "search"}"#
)]
#[case::second_result_ignored(
    r#"{"tasks": [{"status_code": 20000, "result": [{"items": [{"type": "organic", "title": "first"}]}, {"items": [{"type": "organic", "title": "second"}]}]}]}"#,
    r#"{"results": [{"title": "first", "url": "", "snippet": "", "date": null, "last_updated": null}], "object": "search"}"#
)]
#[case::second_task_ignored(
    r#"{"tasks": [{"status_code": 20000, "result": [{"items": [{"type": "organic", "title": "first"}]}]}, {"status_code": 20000, "result": [{"items": [{"type": "organic", "title": "second"}]}]}]}"#,
    r#"{"results": [{"title": "first", "url": "", "snippet": "", "date": null, "last_updated": null}], "object": "search"}"#
)]
fn dataforseo_reply_reads_the_first_organic_result(#[case] payload: &str, #[case] expected: &str) {
    assert_eq!(reply_value(payload), expected_reply(expected));
}

#[rstest]
#[case::missing_tasks("{}")]
#[case::empty_tasks(r#"{"tasks": []}"#)]
#[case::text_status(r#"{"tasks": [{"status_code": "20000", "result": [{"items": [{"type": "organic", "title": "t"}]}]}]}"#)]
#[case::missing_status(
    r#"{"tasks": [{"result": [{"items": [{"type": "organic", "title": "t"}]}]}]}"#
)]
#[case::null_status(r#"{"tasks": [{"status_code": null, "result": [{"items": [{"type": "organic", "title": "t"}]}]}]}"#)]
#[case::failed_status(r#"{"tasks": [{"status_code": 40100, "result": [{"items": [{"type": "organic", "title": "t"}]}]}]}"#)]
#[case::failed_task_then_success(
    r#"{"tasks": [{"status_code": 40100, "result": []}, {"status_code": 20000, "result": [{"items": [{"type": "organic", "title": "second"}]}]}]}"#
)]
#[case::float_status_without_items(r#"{"tasks": [{"status_code": 20000.0, "result": [{}]}]}"#)]
#[case::missing_result(r#"{"tasks": [{"status_code": 20000}]}"#)]
#[case::empty_result(r#"{"tasks": [{"status_code": 20000, "result": []}]}"#)]
#[case::missing_items(r#"{"tasks": [{"status_code": 20000, "result": [{}]}]}"#)]
#[case::empty_items(r#"{"tasks": [{"status_code": 20000, "result": [{"items": []}]}]}"#)]
#[case::unknown_root_members(r#"{"other": 1}"#)]
fn dataforseo_reply_without_a_usable_task_is_empty(#[case] payload: &str) {
    assert_eq!(
        reply_value(payload),
        expected_reply(r#"{"results": [], "object": "search"}"#)
    );
}

#[test]
fn dataforseo_reply_rejects_null_tasks() {
    let error = decode_dataforse_search(br#"{"tasks": null}"#).unwrap_err();
    assert!(matches!(error, DataForSeoSearchError::TasksShape));
}

#[test]
fn dataforseo_reply_rejects_a_null_result() {
    let payload = br#"{"tasks": [{"status_code": 20000, "result": null}]}"#;
    let error = decode_dataforse_search(payload).unwrap_err();
    assert!(matches!(error, DataForSeoSearchError::ResultShape));
}

#[test]
fn dataforseo_reply_rejects_null_items() {
    let payload = br#"{"tasks": [{"status_code": 20000, "result": [{"items": null}]}]}"#;
    let error = decode_dataforse_search(payload).unwrap_err();
    assert!(matches!(error, DataForSeoSearchError::ItemsShape));
}

#[test]
fn dataforseo_reply_ignores_a_null_result_of_a_failed_task() {
    let payload = br#"{"tasks": [{"status_code": 40100, "result": null}]}"#;
    assert_eq!(
        reply_value(std::str::from_utf8(payload).unwrap()),
        expected_reply(r#"{"results": [], "object": "search"}"#)
    );
}

#[rstest]
#[case::scalar_tasks(r#"{"tasks": 5}"#)]
#[case::scalar_result(r#"{"tasks": [{"status_code": 20000, "result": {}}]}"#)]
#[case::scalar_items(r#"{"tasks": [{"status_code": 20000, "result": [{"items": 5}]}]}"#)]
#[case::invalid_json("{")]
#[case::trailing_bytes(r#"{"tasks": []}{"#)]
fn dataforseo_reply_rejects_a_wrong_shape(#[case] payload: &str) {
    let error = decode_dataforse_search(payload.as_bytes()).unwrap_err();
    assert!(matches!(error, DataForSeoSearchError::ReplyDecode(_)));
}

#[rstest]
#[case::array_root("[]")]
#[case::null_root("null")]
#[case::empty_root("")]
#[case::scalar_task(r#"{"tasks": [5]}"#)]
#[case::scalar_result_element(r#"{"tasks": [{"status_code": 20000, "result": [5]}]}"#)]
#[case::null_result_element(r#"{"tasks": [{"status_code": 20000, "result": [null]}]}"#)]
#[case::scalar_item(r#"{"tasks": [{"status_code": 20000, "result": [{"items": [5]}]}]}"#)]
#[case::null_item(r#"{"tasks": [{"status_code": 20000, "result": [{"items": [null]}]}]}"#)]
fn dataforseo_reply_rejects_a_value_that_is_not_an_object(#[case] payload: &str) {
    let error = decode_dataforse_search(payload.as_bytes()).unwrap_err();
    assert!(matches!(error, DataForSeoSearchError::NotAnObject));
}

#[rstest]
#[case::null_title(
    r#"{"tasks": [{"status_code": 20000, "result": [{"items": [{"type": "organic", "title": null}]}]}]}"#,
    "title"
)]
#[case::null_url(
    r#"{"tasks": [{"status_code": 20000, "result": [{"items": [{"type": "organic", "url": null}]}]}]}"#,
    "url"
)]
#[case::null_description(
    r#"{"tasks": [{"status_code": 20000, "result": [{"items": [{"type": "organic", "description": null}]}]}]}"#,
    "description"
)]
fn dataforseo_reply_rejects_a_null_member(#[case] payload: &str, #[case] expected: &str) {
    let error = decode_dataforse_search(payload.as_bytes()).unwrap_err();
    assert!(matches!(error, DataForSeoSearchError::ResultField { field } if field == expected));
}

#[rstest]
#[case::numeric_title(
    r#"{"tasks": [{"status_code": 20000, "result": [{"items": [{"type": "organic", "title": 5}]}]}]}"#,
    "title"
)]
#[case::numeric_url(
    r#"{"tasks": [{"status_code": 20000, "result": [{"items": [{"type": "organic", "url": false}]}]}]}"#,
    "url"
)]
#[case::object_description(
    r#"{"tasks": [{"status_code": 20000, "result": [{"items": [{"type": "organic", "description": {"a": 1}}]}]}]}"#,
    "description"
)]
fn dataforseo_reply_rejects_a_member_of_the_wrong_type(
    #[case] payload: &str,
    #[case] expected: &str,
) {
    let error = decode_dataforse_search(payload.as_bytes()).unwrap_err();
    assert!(matches!(error, DataForSeoSearchError::ReplyField { field, .. } if field == expected));
}

#[test]
fn dataforseo_reply_rejects_a_reply_over_the_bound() {
    let oversized = vec![b' '; REPLY_MAX_BYTES + 1];
    assert!(matches!(
        decode_dataforse_search(&oversized),
        Err(DataForSeoSearchError::ReplyTooLarge { .. })
    ));
}

#[test]
fn dataforseo_reply_accepts_a_reply_at_the_bound() {
    let mut at_bound = vec![b' '; REPLY_MAX_BYTES];
    at_bound[..2].copy_from_slice(b"{}");
    let response = decode_dataforse_search(&at_bound).unwrap();
    assert!(response.results.is_empty());
}

#[rstest]
fn replays_the_dataforseo_corpus(#[files("fuzz/corpus/dataforseo_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_dataforse_search(&bytes));
}
