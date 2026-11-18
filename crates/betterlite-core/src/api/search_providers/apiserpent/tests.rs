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

use super::super::google_pse::wire::SearchParam;
use super::reply::{REPLY_MAX_BYTES, decode_apiserpent_search};
use super::{APISERPENT_API_BASE, ApiserpentBody, ApiserpentSearchConfig, ApiserpentSearchError};

fn environment(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn config(values: &[(&str, &str)]) -> ApiserpentSearchConfig {
    ApiserpentSearchConfig::load(&environment(values))
}

fn optional_params(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn query(text: &str) -> SearchQuery {
    serde_json::from_str(text).unwrap()
}

fn expected(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

fn params_of(record: &IndexMap<String, Json>) -> &IndexMap<String, Json> {
    let Json::Object(params) = &record["_apiserpent_params"] else {
        panic!("the body holds the apiserpent params");
    };
    params
}

fn param(record: &IndexMap<String, Json>, name: &str) -> Json {
    params_of(record)[name].clone()
}

fn request_body(query_text: &str, options_json: &str) -> IndexMap<String, Json> {
    let record = config(&[])
        .transform_search_request(&query(query_text), &optional_params(options_json))
        .unwrap();
    expected(&serde_json::to_string(&record).unwrap())
}

fn request_error(query_text: &str, options_json: &str) -> ApiserpentSearchError {
    config(&[])
        .transform_search_request(&query(query_text), &optional_params(options_json))
        .unwrap_err()
}

fn complete(api_base: Option<&str>, options_json: &str, values: &[(&str, &str)]) -> String {
    let params = optional_params(options_json);
    let config = config(values);
    let record = config
        .transform_search_request(&query("\"q\""), &params)
        .unwrap();
    config
        .complete_url(api_base, &params, Some(&record))
        .unwrap()
}

fn reply(payload: &str) -> IndexMap<String, Json> {
    let response = decode_apiserpent_search(payload.as_bytes()).unwrap();
    expected(&serde_json::to_string(&response).unwrap())
}

#[test]
fn apiserpent_reports_its_display_name_and_get_method() {
    assert_eq!(ApiserpentSearchConfig::ui_friendly_name(), "APISerpent");
    assert_eq!(config(&[]).http_method(), HttpMethod::Get);
}

#[rstest]
#[case::unset(&[], None, APISERPENT_API_BASE)]
#[case::key(&[("APISERPENT_API_KEY", "sk-apiserpent")], Some("sk-apiserpent"), APISERPENT_API_BASE)]
#[case::empty_key(&[("APISERPENT_API_KEY", "")], None, APISERPENT_API_BASE)]
#[case::base(&[("APISERPENT_API_BASE", "https://proxy.test")], None, "https://proxy.test")]
#[case::empty_base(&[("APISERPENT_API_BASE", "")], None, APISERPENT_API_BASE)]
#[case::both(
    &[("APISERPENT_API_KEY", "sk-apiserpent"), ("APISERPENT_API_BASE", "https://proxy.test")],
    Some("sk-apiserpent"),
    "https://proxy.test"
)]
fn apiserpent_config_loads_the_shared_settings(
    #[case] values: &[(&str, &str)],
    #[case] expected_key: Option<&str>,
    #[case] expected_base: &str,
) {
    let config = config(values);
    assert_eq!(
        config.api_key.as_ref().map(ExposeSecret::expose_secret),
        expected_key
    );
    assert_eq!(config.api_base, expected_base);
}

#[test]
fn apiserpent_headers_keep_the_caller_key_and_the_content_type() {
    let config = config(&[("APISERPENT_API_KEY", "sk-server")]);
    let mut headers = Headers::new();
    headers.insert("A".to_owned(), "b".to_owned());
    let key = SecretString::from("sk-caller");
    let validated = config
        .validated_headers(headers, Some(&key), Some("https://evil.test"))
        .unwrap();
    assert_eq!(
        validated.get("X-API-Key").map(String::as_str),
        Some("sk-caller")
    );
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(validated.get("A").map(String::as_str), Some("b"));
}

#[rstest]
#[case::default_host(None, &[("APISERPENT_API_KEY", "sk-server")], true)]
#[case::default_path(
    Some("https://apiserpent.com/api/search/quick"),
    &[("APISERPENT_API_KEY", "sk-server")],
    true
)]
#[case::env_host(
    Some("https://proxy.test/api/search"),
    &[("APISERPENT_API_KEY", "sk-server"), ("APISERPENT_API_BASE", "https://proxy.test")],
    true
)]
#[case::other_host(Some("https://evil.test"), &[("APISERPENT_API_KEY", "sk-server")], false)]
#[case::other_port(Some("https://apiserpent.com:8443"), &[("APISERPENT_API_KEY", "sk-server")], false)]
#[case::empty_base(Some(""), &[("APISERPENT_API_KEY", "sk-server")], true)]
#[case::no_key(Some("https://evil.test"), &[], false)]
fn apiserpent_environment_key_needs_a_trusted_host(
    #[case] api_base: Option<&str>,
    #[case] values: &[(&str, &str)],
    #[case] trusted: bool,
) {
    let validated = config(values).validated_headers(Headers::new(), None, api_base);
    assert_eq!(validated.is_ok(), trusted, "{validated:?}");
}

#[test]
fn apiserpent_untrusted_base_keeps_the_server_key_out_of_the_error() {
    let config = config(&[("APISERPENT_API_KEY", "sk-server")]);
    let error = config
        .validated_headers(Headers::new(), None, Some("https://evil.test"))
        .unwrap_err();
    assert!(matches!(
        error,
        ApiserpentSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
    assert!(!error.to_string().contains("sk-server"));
}

#[test]
fn apiserpent_missing_key_reports_the_source_message() {
    let error = config(&[])
        .validated_headers(Headers::new(), None, None)
        .unwrap_err();
    assert!(matches!(error, ApiserpentSearchError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "APISERPENT_API_KEY is not set. Set `APISERPENT_API_KEY` environment variable."
    );
}

#[test]
fn apiserpent_sign_request_fails_before_sending_without_a_key() {
    let config = config(&[]);
    let record = config
        .transform_search_request(&query("\"q\""), &Object::new())
        .unwrap();
    let empty = Object::new();
    let outbound = OutboundSearch {
        api_base: "https://apiserpent.com/api/search/quick",
        api_key: None,
        optional_params: &empty,
        body: &record,
    };
    let error = config.sign_request(Headers::new(), &outbound).unwrap_err();
    assert!(matches!(error, ApiserpentSearchError::MissingApiKey));
}

#[test]
fn apiserpent_sign_request_blocks_the_server_key_for_an_untrusted_base() {
    let config = config(&[("APISERPENT_API_KEY", "sk-server")]);
    let record = config
        .transform_search_request(&query("\"q\""), &Object::new())
        .unwrap();
    let empty = Object::new();
    let outbound = OutboundSearch {
        api_base: "https://evil.test/api/search/quick",
        api_key: None,
        optional_params: &empty,
        body: &record,
    };
    let error = config.sign_request(Headers::new(), &outbound).unwrap_err();
    assert!(matches!(
        error,
        ApiserpentSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
}

#[rstest]
#[case::server_key_default_base("https://apiserpent.com/api/search/quick", None, "sk-server")]
#[case::caller_key_other_base("https://evil.test/api/search/quick", Some("sk-caller"), "sk-caller")]
fn apiserpent_sign_request_signs_with_the_key_and_content_type(
    #[case] api_base: &str,
    #[case] caller_key: Option<&str>,
    #[case] expected_key: &str,
) {
    let config = config(&[("APISERPENT_API_KEY", "sk-server")]);
    let record = config
        .transform_search_request(&query("\"q\""), &Object::new())
        .unwrap();
    let caller_secret = caller_key.map(SecretString::from);
    let empty = Object::new();
    let outbound = OutboundSearch {
        api_base,
        api_key: caller_secret.as_ref(),
        optional_params: &empty,
        body: &record,
    };
    let (headers, signed_body) = config.sign_request(Headers::new(), &outbound).unwrap();
    assert_eq!(
        headers.get("X-API-Key").map(String::as_str),
        Some(expected_key)
    );
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(signed_body, None);
}

#[test]
fn apiserpent_validate_environment_adds_the_key_and_content_type() {
    let config = config(&[("APISERPENT_API_KEY", "sk-server")]);
    let mut headers = Headers::new();
    headers.insert("A".to_owned(), "b".to_owned());
    let validated = config.validate_environment(headers, None, None);
    assert_eq!(
        validated.get("X-API-Key").map(String::as_str),
        Some("sk-server")
    );
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(validated.get("A").map(String::as_str), Some("b"));
}

#[test]
fn apiserpent_validate_environment_leaves_headers_alone_on_a_missing_key() {
    let config = config(&[]);
    let mut headers = Headers::new();
    headers.insert("A".to_owned(), "b".to_owned());
    let kept = config.validate_environment(headers.clone(), None, None);
    assert_eq!(kept, headers);
}

#[rstest]
#[case::default("{}", "https://apiserpent.com/api/search/quick")]
#[case::deep_true(r#"{"deep": true}"#, "https://apiserpent.com/api/search")]
#[case::deep_false(r#"{"deep": false}"#, "https://apiserpent.com/api/search/quick")]
#[case::deep_string_false(r#"{"deep": "false"}"#, "https://apiserpent.com/api/search")]
#[case::deep_empty_string(r#"{"deep": ""}"#, "https://apiserpent.com/api/search/quick")]
#[case::deep_zero(r#"{"deep": 0}"#, "https://apiserpent.com/api/search/quick")]
#[case::deep_one(r#"{"deep": 1}"#, "https://apiserpent.com/api/search")]
#[case::deep_empty_list(r#"{"deep": []}"#, "https://apiserpent.com/api/search/quick")]
#[case::deep_empty_object(r#"{"deep": {}}"#, "https://apiserpent.com/api/search/quick")]
#[case::deep_null(r#"{"deep": null}"#, "https://apiserpent.com/api/search/quick")]
fn apiserpent_url_switches_between_quick_and_deep(
    #[case] options_json: &str,
    #[case] expected_url: &str,
) {
    let params = optional_params(options_json);
    let complete_url = config(&[]).complete_url(None, &params, None).unwrap();
    assert_eq!(complete_url, expected_url);
}

#[rstest]
#[case::default(None, "{}", &[], "https://apiserpent.com/api/search/quick")]
#[case::caller_base(Some("https://x.test"), "{}", &[], "https://x.test/api/search/quick")]
#[case::trailing_slash(Some("https://x.test/"), "{}", &[], "https://x.test/api/search/quick")]
#[case::double_slash(Some("https://x.test//"), "{}", &[], "https://x.test/api/search/quick")]
#[case::already_quick(
    Some("https://x.test/api/search/quick"),
    "{}",
    &[],
    "https://x.test/api/search/quick"
)]
#[case::quick_slash(
    Some("https://x.test/api/search/quick/"),
    "{}",
    &[],
    "https://x.test/api/search/quick"
)]
#[case::already_deep(
    Some("https://x.test/api/search"),
    r#"{"deep": true}"#,
    &[],
    "https://x.test/api/search"
)]
#[case::quick_base_with_deep(
    Some("https://x.test/api/search/quick"),
    r#"{"deep": true}"#,
    &[],
    "https://x.test/api/search/quick/api/search"
)]
#[case::deep_base_with_quick(
    Some("https://x.test/api/search"),
    "{}",
    &[],
    "https://x.test/api/search/api/search/quick"
)]
#[case::bare_slash(Some("/"), "{}", &[], "/api/search/quick")]
#[case::env_base(
    None,
    "{}",
    &[("APISERPENT_API_BASE", "https://env.test/")],
    "https://env.test/api/search/quick"
)]
#[case::empty_caller(Some(""), "{}", &[], "https://apiserpent.com/api/search/quick")]
#[case::empty_env(
    None,
    "{}",
    &[("APISERPENT_API_BASE", "")],
    "https://apiserpent.com/api/search/quick"
)]
fn apiserpent_url_trims_trailing_slashes(
    #[case] api_base: Option<&str>,
    #[case] options_json: &str,
    #[case] values: &[(&str, &str)],
    #[case] expected_url: &str,
) {
    let params = optional_params(options_json);
    let complete_url = config(values)
        .complete_url(api_base, &params, None)
        .unwrap();
    assert_eq!(complete_url, expected_url);
}

#[test]
fn apiserpent_url_repeats_a_list_param_with_doseq() {
    let mut params = IndexMap::new();
    params.insert(
        "language".to_owned(),
        SearchParam::Many(vec![
            SearchParam::Text("en".to_owned()),
            SearchParam::Text("fr".to_owned()),
        ]),
    );
    let record = ApiserpentBody { params };
    let complete_url = config(&[])
        .complete_url(None, &Object::new(), Some(&record))
        .unwrap();
    assert_eq!(
        complete_url,
        "https://apiserpent.com/api/search/quick?language=en&language=fr"
    );
}

#[test]
fn apiserpent_url_appends_the_encoded_params() {
    assert_eq!(
        complete(None, "{}", &[]),
        "https://apiserpent.com/api/search/quick?engine=google&country=us&num=10&format=full&q=q"
    );
}

#[test]
fn apiserpent_url_encodes_the_query_characters() {
    assert_eq!(
        complete(None, r#"{"search_domain_filter": ["a.com", "b.com"]}"#, &[]),
        "https://apiserpent.com/api/search/quick?engine=google&country=us&num=10&format=full&q=%28q%29+%28site%3Aa.com+OR+site%3Ab.com%29"
    );
}

#[test]
fn apiserpent_request_defaults_to_google_us_ten_full() {
    assert_eq!(
        request_body("\"q\"", "{}"),
        expected(
            r#"{"_apiserpent_params": {"engine": "google", "country": "us", "num": 10, "format": "full", "q": "q"}}"#
        )
    );
}

#[test]
fn apiserpent_request_joins_a_list_query() {
    assert_eq!(
        request_body("[\"a\", \"b\"]", "{}"),
        expected(
            r#"{"_apiserpent_params": {"engine": "google", "country": "us", "num": 10, "format": "full", "q": "a b"}}"#
        )
    );
}

#[rstest]
#[case::quick_zero(r#"{"max_results": 0}"#, 1)]
#[case::quick_one(r#"{"max_results": 1}"#, 1)]
#[case::quick_hundred(r#"{"max_results": 100}"#, 100)]
#[case::quick_over(r#"{"max_results": 500}"#, 100)]
#[case::deep_floor(r#"{"deep": true, "max_results": 2}"#, 10)]
#[case::deep_inside(r#"{"deep": true, "max_results": 50}"#, 50)]
#[case::deep_over(r#"{"deep": true, "max_results": 500}"#, 100)]
#[case::null_max_results(r#"{"max_results": null, "num": 4}"#, 4)]
fn apiserpent_request_clamps_max_results(#[case] options_json: &str, #[case] expected_num: i64) {
    let record = request_body("\"q\"", options_json);
    assert_eq!(
        param(&record, "num"),
        Json::Number(serde_json::Number::from(expected_num))
    );
}

#[rstest]
#[case::quick_two(r#"{"num": 2}"#, 2)]
#[case::deep_two_below_floor(r#"{"deep": true, "num": 2}"#, 2)]
#[case::deep_hundred(r#"{"deep": true, "num": 100}"#, 100)]
#[case::null_num(r#"{"num": null}"#, 10)]
fn apiserpent_request_keeps_an_explicit_num(#[case] options_json: &str, #[case] expected_num: i64) {
    let record = request_body("\"q\"", options_json);
    assert_eq!(
        param(&record, "num"),
        Json::Number(serde_json::Number::from(expected_num))
    );
}

#[test]
fn apiserpent_request_lets_max_results_win_over_an_explicit_num() {
    let quick = request_body("\"q\"", r#"{"max_results": 5, "num": 3}"#);
    assert_eq!(
        param(&quick, "num"),
        Json::Number(serde_json::Number::from(5_i64))
    );
    let deep = request_body("\"q\"", r#"{"deep": true, "max_results": 20, "num": 3}"#);
    assert_eq!(
        param(&deep, "num"),
        Json::Number(serde_json::Number::from(20_i64))
    );
}

#[rstest]
#[case::num_zero(r#"{"num": 0}"#, "num must be between 1 and 100, got 0")]
#[case::num_over(r#"{"num": 101}"#, "num must be between 1 and 100, got 101")]
#[case::pages_zero(r#"{"pages": 0}"#, "pages must be between 1 and 10, got 0")]
#[case::pages_over(r#"{"pages": 11}"#, "pages must be between 1 and 10, got 11")]
fn apiserpent_request_rejects_a_value_out_of_range(
    #[case] options_json: &str,
    #[case] expected_message: &str,
) {
    let error = request_error("\"q\"", options_json);
    assert_eq!(error.to_string(), expected_message);
}

#[rstest]
#[case::pages_one(r#"{"pages": 1}"#, Some(1))]
#[case::pages_ten(r#"{"pages": 10}"#, Some(10))]
#[case::null_pages(r#"{"pages": null}"#, None)]
fn apiserpent_request_accepts_or_skips_pages(
    #[case] options_json: &str,
    #[case] expected_pages: Option<i64>,
) {
    let record = request_body("\"q\"", options_json);
    match expected_pages {
        Some(pages) => assert_eq!(
            param(&record, "pages"),
            Json::Number(serde_json::Number::from(pages))
        ),
        None => assert!(!params_of(&record).contains_key("pages")),
    }
}

#[test]
fn apiserpent_request_lowercases_country() {
    assert_eq!(
        param(&request_body("\"q\"", r#"{"country": "US"}"#), "country"),
        Json::Text("us".to_owned())
    );
    assert_eq!(
        param(&request_body("\"q\"", r#"{"country": ""}"#), "country"),
        Json::Text(String::new())
    );
    assert_eq!(
        param(&request_body("\"q\"", r#"{"country": null}"#), "country"),
        Json::Text("us".to_owned())
    );
}

#[test]
fn apiserpent_request_copies_recognized_fields_without_overwriting_mapped_ones() {
    let record = request_body(
        "\"q\"",
        r#"{"engine": "bing", "format": "simple", "pages": 3, "freshness": "d", "safe": "strict", "language": "en", "pixel_position": true, "deep": true, "max_results": 5, "q": "ignored", "unknown": 1}"#,
    );
    assert_eq!(
        record,
        expected(
            r#"{"_apiserpent_params": {"engine": "bing", "country": "us", "num": 5, "format": "simple", "pages": 3, "freshness": "d", "safe": "strict", "language": "en", "pixel_position": "true", "q": "q"}}"#
        )
    );
}

#[test]
fn apiserpent_request_lowercases_a_boolean_pixel_position() {
    let record = request_body("\"q\"", r#"{"pixel_position": false}"#);
    assert_eq!(
        param(&record, "pixel_position"),
        Json::Text("false".to_owned())
    );
}

#[test]
fn apiserpent_request_accepts_literal_values_unchecked() {
    let record = request_body(
        "\"q\"",
        r#"{"engine": "yandex", "format": "verbose", "safe": "kind", "freshness": "weird", "language": "xx"}"#,
    );
    assert_eq!(
        record,
        expected(
            r#"{"_apiserpent_params": {"engine": "yandex", "country": "us", "num": 10, "format": "verbose", "freshness": "weird", "safe": "kind", "language": "xx", "q": "q"}}"#
        )
    );
}

#[rstest]
#[case::engine(r#"{"engine": 5}"#, "engine")]
#[case::country(r#"{"country": 5}"#, "country")]
#[case::format(r#"{"format": 5}"#, "format")]
#[case::freshness(r#"{"freshness": 5}"#, "freshness")]
#[case::safe(r#"{"safe": 5}"#, "safe")]
#[case::language(r#"{"language": 5}"#, "language")]
#[case::pages(r#"{"pages": "3"}"#, "pages")]
#[case::num(r#"{"num": "10"}"#, "num")]
#[case::pixel_position(r#"{"pixel_position": "true"}"#, "pixel_position")]
#[case::max_results(r#"{"max_results": "10"}"#, "max_results")]
fn apiserpent_request_rejects_a_wrong_typed_option(
    #[case] options_json: &str,
    #[case] expected_field: &'static str,
) {
    let error = request_error("\"q\"", options_json);
    assert!(
        matches!(&error, ApiserpentSearchError::OptionType { field } if *field == expected_field)
    );
}

#[test]
fn apiserpent_request_appends_domain_clauses() {
    assert_eq!(
        param(
            &request_body("\"q\"", r#"{"search_domain_filter": ["a.com", "b.com"]}"#),
            "q"
        ),
        Json::Text("(q) (site:a.com OR site:b.com)".to_owned())
    );
    assert_eq!(
        param(
            &request_body("\"q\"", r#"{"search_domain_filter": ["a.com"]}"#),
            "q"
        ),
        Json::Text("(q) (site:a.com)".to_owned())
    );
    assert_eq!(
        param(
            &request_body("[\"a\", \"b\"]", r#"{"search_domain_filter": ["a.com"]}"#),
            "q"
        ),
        Json::Text("(a b) (site:a.com)".to_owned())
    );
}

#[rstest]
#[case::empty(r#"{"search_domain_filter": []}"#)]
#[case::text(r#"{"search_domain_filter": "a.com"}"#)]
#[case::null(r#"{"search_domain_filter": null}"#)]
fn apiserpent_request_ignores_a_domain_filter_that_is_not_a_nonempty_list(
    #[case] options_json: &str,
) {
    assert_eq!(
        param(&request_body("\"q\"", options_json), "q"),
        Json::Text("q".to_owned())
    );
}

#[test]
fn apiserpent_request_rejects_a_non_string_domain() {
    let error = request_error("\"q\"", r#"{"search_domain_filter": [1]}"#);
    assert!(matches!(
        error,
        ApiserpentSearchError::OptionList {
            field: "search_domain_filter"
        }
    ));
}

#[rstest]
#[case::flat(
    r#"{"results": [{"title": "t", "url": "u", "snippet": "s", "date": "2024-01-02"}]}"#,
    r#"{"results": [{"title": "t", "url": "u", "snippet": "s", "date": "2024-01-02", "last_updated": null}], "object": "search"}"#
)]
#[case::organic(
    r#"{"results": {"organic": [{"title": "t", "url": "u", "snippet": "s"}]}}"#,
    r#"{"results": [{"title": "t", "url": "u", "snippet": "s", "date": null, "last_updated": null}], "object": "search"}"#
)]
#[case::missing_members(
    r#"{"results": [{}]}"#,
    r#"{"results": [{"title": "", "url": "", "snippet": "", "date": null, "last_updated": null}], "object": "search"}"#
)]
#[case::unknown_members(
    r#"{"results": [{"title": "t", "position": 1}]}"#,
    r#"{"results": [{"title": "t", "url": "", "snippet": "", "date": null, "last_updated": null}], "object": "search"}"#
)]
#[case::null_date(
    r#"{"results": [{"date": null}]}"#,
    r#"{"results": [{"title": "", "url": "", "snippet": "", "date": null, "last_updated": null}], "object": "search"}"#
)]
fn apiserpent_reply_reads_both_result_shapes(#[case] payload: &str, #[case] expected_reply: &str) {
    assert_eq!(reply(payload), expected(expected_reply));
}

#[rstest]
#[case::missing("{}")]
#[case::null(r#"{"results": null}"#)]
#[case::empty_list(r#"{"results": []}"#)]
#[case::empty_object(r#"{"results": {}}"#)]
#[case::false_value(r#"{"results": false}"#)]
#[case::zero(r#"{"results": 0}"#)]
#[case::zero_float(r#"{"results": 0.0}"#)]
#[case::empty_string(r#"{"results": ""}"#)]
fn apiserpent_reply_turns_falsey_results_into_nothing(#[case] payload: &str) {
    assert_eq!(
        reply(payload),
        expected(r#"{"results": [], "object": "search"}"#)
    );
}

#[rstest]
#[case::missing_organic(r#"{"results": {"other": 1}}"#)]
#[case::empty_organic(r#"{"results": {"organic": []}}"#)]
#[case::empty_organic_object(r#"{"results": {"organic": {}}}"#)]
#[case::empty_organic_string(r#"{"results": {"organic": ""}}"#)]
fn apiserpent_reply_reads_an_empty_organic_list(#[case] payload: &str) {
    assert_eq!(
        reply(payload),
        expected(r#"{"results": [], "object": "search"}"#)
    );
}

#[test]
fn apiserpent_reply_rejects_null_organic() {
    let error = decode_apiserpent_search(br#"{"results": {"organic": null}}"#).unwrap_err();
    assert!(matches!(error, ApiserpentSearchError::OrganicShape));
}

#[rstest]
#[case::organic_string(r#"{"results": {"organic": "abc"}}"#)]
#[case::organic_object(r#"{"results": {"organic": {"a": 1}}}"#)]
#[case::organic_number(r#"{"results": {"organic": 0}}"#)]
#[case::organic_false(r#"{"results": {"organic": false}}"#)]
#[case::organic_true(r#"{"results": {"organic": true}}"#)]
#[case::results_string(r#"{"results": "abc"}"#)]
#[case::results_number(r#"{"results": 5}"#)]
#[case::results_true(r#"{"results": true}"#)]
fn apiserpent_reply_rejects_a_truthy_value_that_cannot_hold_results(#[case] payload: &str) {
    assert!(decode_apiserpent_search(payload.as_bytes()).is_err());
}

#[rstest]
#[case::null_title(r#"{"results": [{"title": null}]}"#)]
#[case::null_url(r#"{"results": [{"url": null}]}"#)]
#[case::null_snippet(r#"{"results": [{"snippet": null}]}"#)]
#[case::numeric_title(r#"{"results": [{"title": 5}]}"#)]
#[case::numeric_url(r#"{"results": [{"url": 5}]}"#)]
#[case::numeric_snippet(r#"{"results": [{"snippet": 5}]}"#)]
#[case::numeric_date(r#"{"results": [{"date": 5}]}"#)]
#[case::null_member(r#"{"results": [null]}"#)]
#[case::array_member(r#"{"results": [[]]}"#)]
fn apiserpent_reply_rejects_an_invalid_member(#[case] payload: &str) {
    assert!(decode_apiserpent_search(payload.as_bytes()).is_err());
}

#[rstest]
#[case::invalid_json("{")]
#[case::trailing_bytes(r#"{"results": []}{"#)]
fn apiserpent_reply_rejects_malformed_json(#[case] payload: &str) {
    let error = decode_apiserpent_search(payload.as_bytes()).unwrap_err();
    assert!(matches!(error, ApiserpentSearchError::ReplyDecode(_)));
}

#[rstest]
#[case::array("[]")]
#[case::null("null")]
#[case::string("\"abc\"")]
#[case::number("5")]
#[case::empty("")]
fn apiserpent_reply_rejects_a_reply_that_is_not_an_object(#[case] payload: &str) {
    let error = decode_apiserpent_search(payload.as_bytes()).unwrap_err();
    assert!(matches!(error, ApiserpentSearchError::NotAnObject));
}

#[test]
fn apiserpent_reply_rejects_a_reply_over_the_bound() {
    let oversized = vec![b' '; REPLY_MAX_BYTES + 1];
    assert!(matches!(
        decode_apiserpent_search(&oversized),
        Err(ApiserpentSearchError::ReplyTooLarge { .. })
    ));
}

#[test]
fn apiserpent_reply_accepts_a_reply_at_the_bound() {
    let mut at_bound = vec![b' '; REPLY_MAX_BYTES];
    at_bound[..2].copy_from_slice(b"{}");
    let response = decode_apiserpent_search(&at_bound).unwrap();
    assert!(response.results.is_empty());
}

#[rstest]
fn replays_the_apiserpent_corpus(#[files("fuzz/corpus/apiserpent_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_apiserpent_search(&bytes));
}
