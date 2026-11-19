#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::CredentialError;
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, ProviderResponse, SearchProvider as _, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use chrono::NaiveDate;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::dates::{self, DateFlags, DateValue};
use super::reply::REPLY_MAX_BYTES;
use super::{BRAVE_API_BASE, BraveSearchConfig, BraveSearchError};

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 3, 5).unwrap()
}

fn config(variables: &[(&str, &str)]) -> BraveSearchConfig {
    BraveSearchConfig::load(&environment(variables), Some(today()))
}

fn options(pairs: &[(&str, JsonValue)]) -> Object {
    Object::from_iter(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone())),
    )
}

fn text(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}

fn number(value: i64) -> JsonValue {
    JsonValue::from(value)
}

fn query(value: &str) -> SearchQuery {
    SearchQuery::Single(value.to_owned())
}

fn request_body(config: &BraveSearchConfig, value: &str, params: &Object) -> JsonValue {
    let body = config
        .transform_search_request(&query(value), params)
        .unwrap();
    serde_json::to_value(body).unwrap()
}

fn request_url(config: &BraveSearchConfig, value: &str, params: &Object) -> String {
    let body = config
        .transform_search_request(&query(value), params)
        .unwrap();
    config.complete_url(None, params, Some(&body)).unwrap()
}

fn answer(
    config: &BraveSearchConfig,
    value: &str,
    params: &Object,
    body: &str,
) -> Result<SearchResponse, BraveSearchError> {
    let _request = config
        .transform_search_request(&query(value), params)
        .unwrap();
    let _url = config.complete_url(None, params, Some(&_request)).unwrap();
    let headers = Headers::new();
    config.transform_search_response(&ProviderResponse {
        status_code: 200,
        headers: &headers,
        text: body,
    })
}

fn date_of(literal: &str) -> Option<String> {
    dates::to_yyyy_mm_dd(
        &DateValue::from_literal(literal),
        today(),
        DateFlags::default(),
    )
}

#[test]
fn brave_search_settings_default_the_key_to_unset_and_the_base_to_the_source() {
    let config = config(&[]);

    assert!(config.api_key.is_none());
    assert_eq!(config.api_base, BRAVE_API_BASE);
}

#[test]
fn brave_search_settings_read_the_environment() {
    let config = config(&[
        ("BRAVE_API_KEY", "server-key"),
        ("BRAVE_API_BASE", "https://proxy.test/res/v1/web/search"),
    ]);

    assert_eq!(
        config.api_key.as_ref().map(|key| key.expose_secret()),
        Some("server-key")
    );
    assert_eq!(config.api_base, "https://proxy.test/res/v1/web/search");
}

#[test]
fn brave_search_settings_treat_an_empty_variable_as_unset() {
    let config = config(&[("BRAVE_API_KEY", ""), ("BRAVE_API_BASE", "")]);

    assert!(config.api_key.is_none());
    assert_eq!(config.api_base, BRAVE_API_BASE);
}

#[test]
fn brave_search_is_a_named_get_provider() {
    assert_eq!("Brave Search", BraveSearchConfig::ui_friendly_name());
    assert_eq!(HttpMethod::Get, config(&[]).http_method());
}

#[test]
fn brave_search_headers_keep_the_callers_beside_the_source_ones() {
    let config = config(&[("BRAVE_API_KEY", "server-key")]);
    let mut headers = Headers::new();
    headers.insert("X-Caller".to_owned(), "1".to_owned());

    let validated = config.validate_environment(headers, None, None);

    assert_eq!(validated.get("X-Caller").map(String::as_str), Some("1"));
    assert_eq!(
        validated.get("X-Subscription-Token").map(String::as_str),
        Some("server-key")
    );
    assert_eq!(
        validated.get("Accept").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        validated.get("Accept-Encoding").map(String::as_str),
        Some("gzip")
    );
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn brave_search_sign_request_fails_before_sending_without_a_key() {
    let config = config(&[]);
    let body = config
        .transform_search_request(&query("q"), &Object::new())
        .unwrap();
    let empty = Object::new();
    let outbound = OutboundSearch {
        api_base: BRAVE_API_BASE,
        api_key: None,
        optional_params: &empty,
        body: &body,
    };

    let error = config.sign_request(Headers::new(), &outbound).unwrap_err();

    assert!(matches!(error, BraveSearchError::MissingApiKey));
}

#[test]
fn brave_search_sign_request_blocks_the_server_key_for_an_untrusted_base() {
    let config = config(&[("BRAVE_API_KEY", "server-key")]);
    let body = config
        .transform_search_request(&query("q"), &Object::new())
        .unwrap();
    let empty = Object::new();
    let outbound = OutboundSearch {
        api_base: "https://evil.test/search",
        api_key: None,
        optional_params: &empty,
        body: &body,
    };

    let error = config.sign_request(Headers::new(), &outbound).unwrap_err();

    assert!(matches!(
        error,
        BraveSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
}

#[rstest]
#[case::server_key_default_base(BRAVE_API_BASE, None, "server-key")]
#[case::server_key_environment_base("https://proxy.test/res/v1/web/search", None, "server-key")]
#[case::caller_key_other_base("https://evil.test/search", Some("caller-key"), "caller-key")]
fn brave_search_sign_request_picks_the_key_the_policy_allows(
    #[case] api_base: &str,
    #[case] caller_key: Option<&str>,
    #[case] expected_key: &str,
) {
    let config = config(&[
        ("BRAVE_API_KEY", "server-key"),
        ("BRAVE_API_BASE", "https://proxy.test/res/v1/web/search"),
    ]);
    let body = config
        .transform_search_request(&query("q"), &Object::new())
        .unwrap();
    let caller = caller_key.map(SecretString::from);
    let empty = Object::new();
    let outbound = OutboundSearch {
        api_base,
        api_key: caller.as_ref(),
        optional_params: &empty,
        body: &body,
    };

    let (headers, signed) = config.sign_request(Headers::new(), &outbound).unwrap();

    assert_eq!(
        headers.get("X-Subscription-Token").map(String::as_str),
        Some(expected_key)
    );
    assert_eq!(signed, None);
}

#[test]
fn brave_search_request_joins_lists_and_defaults_the_metadata_flag() {
    let config = config(&[]);
    let joined = config
        .transform_search_request(
            &SearchQuery::Multiple(vec!["one".to_owned(), "two".to_owned()]),
            &Object::new(),
        )
        .unwrap();

    assert_eq!(
        serde_json::to_value(joined).unwrap(),
        serde_json::json!({
            "_brave_params": {"q": "one two", "include_fetch_metadata": true}
        })
    );
}

#[rstest]
#[case::explicit_null(JsonValue::Null, true)]
#[case::explicit_true(JsonValue::Bool(true), true)]
#[case::explicit_false(JsonValue::Bool(false), false)]
#[case::zero(JsonValue::from(0), true)]
#[case::text_false(text("false"), true)]
fn brave_search_request_keeps_metadata_unless_false_is_boolean(
    #[case] value: JsonValue,
    #[case] expected: bool,
) {
    let config = config(&[]);
    let params = options(&[("include_fetch_metadata", value)]);

    let body = request_body(&config, "q", &params);

    assert_eq!(body["_brave_params"]["include_fetch_metadata"], expected);
}

#[test]
fn brave_search_request_ignores_the_unified_country_and_copies_other_fields() {
    let config = config(&[]);
    let params = options(&[
        ("country", text("US")),
        ("offset", number(2)),
        ("search_lang", text("en")),
        ("text_decorations", JsonValue::Bool(false)),
    ]);

    let body = request_body(&config, "q", &params);

    assert_eq!(
        body,
        serde_json::json!({
            "_brave_params": {
                "q": "q",
                "include_fetch_metadata": true,
                "offset": 2,
                "search_lang": "en",
                "text_decorations": false
            }
        })
    );
}

#[test]
fn brave_search_request_copies_an_explicit_count_only_once() {
    let config = config(&[]);
    let params = options(&[("count", number(7)), ("max_results", number(5))]);

    let body = request_body(&config, "q", &params);

    assert_eq!(body["_brave_params"]["count"], 5);
}

#[rstest]
#[case::capped_above(25, 20)]
#[case::kept_as_given(7, 7)]
#[case::zero_passes(0, 0)]
#[case::negative_passes(-3, -3)]
fn brave_search_request_caps_max_results_only_above(#[case] given: i64, #[case] sent: i64) {
    let config = config(&[]);
    let params = options(&[("max_results", number(given))]);

    let body = request_body(&config, "q", &params);

    assert_eq!(body["_brave_params"]["count"], sent);
}

#[test]
fn brave_search_request_wraps_domains_into_the_query() {
    let config = config(&[]);
    let params = options(&[(
        "search_domain_filter",
        serde_json::json!(["a.test", "b.test"]),
    )]);

    let body = request_body(&config, "cats", &params);

    assert_eq!(
        body["_brave_params"]["q"],
        "(cats) AND (site:a.test OR site:b.test)"
    );
}

#[test]
fn brave_search_request_keeps_an_empty_domain_list_out_of_the_query() {
    let config = config(&[]);
    let params = options(&[("search_domain_filter", serde_json::json!([]))]);

    let body = request_body(&config, "cats", &params);

    assert_eq!(body["_brave_params"]["q"], "cats");
}

#[test]
fn brave_search_request_rejects_a_max_results_that_is_not_an_integer() {
    let config = config(&[]);
    let params = options(&[("max_results", text("5"))]);

    let error = config
        .transform_search_request(&query("q"), &params)
        .unwrap_err();

    assert!(matches!(
        error,
        BraveSearchError::OptionType { field } if field == "max_results"
    ));
}

#[test]
fn brave_search_request_rejects_a_domain_that_is_not_text() {
    let config = config(&[]);
    let params = options(&[("search_domain_filter", serde_json::json!([5]))]);

    let error = config
        .transform_search_request(&query("q"), &params)
        .unwrap_err();

    assert!(matches!(
        error,
        BraveSearchError::OptionType { field } if field == "search_domain_filter"
    ));
}

#[test]
fn brave_search_request_rejects_a_passthrough_object() {
    let config = config(&[]);
    let params = options(&[("goggles", serde_json::json!({"a": 1}))]);

    let error = config
        .transform_search_request(&query("q"), &params)
        .unwrap_err();

    assert!(matches!(
        error,
        BraveSearchError::OptionType { field } if field == "goggles"
    ));
}

#[test]
fn brave_search_url_appends_the_encoded_parameters_without_touching_the_base() {
    let config = config(&[]);
    let params = options(&[("max_results", number(5))]);

    let url = request_url(&config, "hello world", &params);

    assert_eq!(
        url,
        format!("{BRAVE_API_BASE}?q=hello+world&include_fetch_metadata=True&count=5")
    );
}

#[test]
fn brave_search_url_keeps_the_caller_base_verbatim() {
    let config = config(&[]);
    let params = Object::new();
    let body = config
        .transform_search_request(&query("q"), &params)
        .unwrap();

    let url = config
        .complete_url(Some("https://proxy.test/v1/search/"), &params, Some(&body))
        .unwrap();

    assert_eq!(
        url,
        "https://proxy.test/v1/search/?q=q&include_fetch_metadata=True"
    );
}

#[test]
fn brave_search_reply_reads_every_section_in_the_source_order() {
    let config = config(&[]);
    let body = r#"{
        "videos": {"results": [{"title": "v", "url": "https://v.test", "description": "video"}]},
        "web": {"results": [{"title": "w", "url": "https://w.test", "description": "web"}]},
        "discussions": {"results": [{"title": "d", "url": "https://d.test"}]},
        "faqs": {"results": [{"title": "fs", "url": "https://fs.test"}]},
        "faq": {"results": [{"title": "f", "url": "https://f.test"}]},
        "news": {"results": [{"title": "n", "url": "https://n.test"}]}
    }"#;

    let response = answer(&config, "q", &Object::new(), body).unwrap();

    let titles: Vec<&str> = response
        .results
        .iter()
        .map(|result| result.title.as_str())
        .collect();
    assert_eq!(titles, ["w", "d", "fs", "f", "n", "v"]);
    assert_eq!(response.object.0, "search");
}

#[test]
fn brave_search_reply_filters_sections_case_insensitively_and_keeps_duplicates() {
    let config = config(&[]);
    let params = options(&[("result_filter", text(" NEWS, web ,news,unknown"))]);
    let body = r#"{
        "web": {"results": [{"title": "w", "url": "u"}]},
        "news": {"results": [{"title": "n", "url": "u"}]},
        "videos": {"results": [{"title": "v", "url": "u"}]}
    }"#;

    let response = answer(&config, "q", &params, body).unwrap();

    let titles: Vec<&str> = response
        .results
        .iter()
        .map(|result| result.title.as_str())
        .collect();
    assert_eq!(titles, ["n", "w", "n"]);
}

#[test]
fn brave_search_reply_selects_every_section_for_an_unsupported_filter() {
    let config = config(&[]);
    let params = options(&[("result_filter", text("images"))]);
    let body = r#"{"web": {"results": [{"title": "w", "url": "u"}]}}"#;

    let response = answer(&config, "q", &params, body).unwrap();

    assert_eq!(response.results.len(), 1);
}

#[rstest]
#[case::absent(None, 4)]
#[case::zero(Some(0), 1)]
#[case::one(Some(1), 1)]
#[case::given(Some(3), 3)]
#[case::at_the_limit(Some(20), 4)]
#[case::above_the_limit(Some(99), 4)]
fn brave_search_reply_clamps_the_count_across_sections(
    #[case] max_results: Option<i64>,
    #[case] expected: usize,
) {
    let config = config(&[]);
    let params = match max_results {
        Some(value) => options(&[("max_results", number(value))]),
        None => Object::new(),
    };
    let body = r#"{
        "web": {"results": [{"title": "w1", "url": "u"}, {"title": "w2", "url": "u"}]},
        "news": {"results": [{"title": "n1", "url": "u"}, {"title": "n2", "url": "u"}]}
    }"#;

    let response = answer(&config, "q", &params, body).unwrap();

    assert_eq!(response.results.len(), expected);
}

#[test]
fn brave_search_reply_reads_the_count_a_caller_sent_directly() {
    let config = config(&[]);
    let params = options(&[("count", number(0))]);
    let body =
        r#"{"web": {"results": [{"title": "w1", "url": "u"}, {"title": "w2", "url": "u"}]}}"#;

    let response = answer(&config, "q", &params, body).unwrap();

    assert_eq!(response.results.len(), 1);
    assert!(request_url(&config, "q", &params).contains("count=0"));
}

#[test]
fn brave_search_reply_defaults_missing_text_and_maps_the_fields() {
    let config = config(&[]);
    let body = r#"{
        "web": {"results": [
            {"title": "t", "url": "https://x.test", "description": "s",
             "page_age": "2024-03-05T23:30:00-0500", "age": "2020-01-01",
             "fetched_content_timestamp": 1700000000},
            {}
        ]}
    }"#;

    let response = answer(&config, "q", &Object::new(), body).unwrap();

    assert_eq!(response.results[0].snippet, "s");
    assert_eq!(response.results[0].date.as_deref(), Some("2024-03-05"));
    assert_eq!(
        response.results[0].last_updated.as_deref(),
        Some("2023-11-14")
    );
    assert_eq!(response.results[1].title, "");
    assert_eq!(response.results[1].url, "");
    assert_eq!(response.results[1].snippet, "");
    assert_eq!(response.results[1].date, None);
    assert_eq!(response.results[1].last_updated, None);
}

#[test]
fn brave_search_reply_falls_back_to_age_only_when_page_age_is_empty() {
    let config = config(&[]);
    let body = r#"{
        "web": {"results": [
            {"page_age": 0, "age": "2024-03-05"},
            {"page_age": "", "age": "2024-03-05"},
            {"page_age": "0", "age": "2024-03-05"}
        ]}
    }"#;

    let response = answer(&config, "q", &Object::new(), body).unwrap();

    assert_eq!(response.results[0].date.as_deref(), Some("2024-03-05"));
    assert_eq!(response.results[1].date.as_deref(), Some("2024-03-05"));
    assert_eq!(response.results[2].date.as_deref(), Some("1970-01-01"));
}

#[test]
fn brave_search_reply_stops_reading_after_the_count() {
    let config = config(&[]);
    let params = options(&[("max_results", number(1))]);
    let body = r#"{
        "web": {"results": [
            {"title": "t", "url": "u"},
            {"title": 5, "url": "u"}
        ]}
    }"#;

    let response = answer(&config, "q", &params, body).unwrap();

    assert_eq!(response.results.len(), 1);
}

#[test]
fn brave_search_reply_rejects_a_title_that_is_not_text() {
    let config = config(&[]);
    let body = r#"{"web": {"results": [{"title": 5, "url": "u"}]}}"#;

    let error = answer(&config, "q", &Object::new(), body).unwrap_err();

    assert!(matches!(
        error,
        BraveSearchError::ReplyText { field: "title", .. }
    ));
}

#[test]
fn brave_search_reply_treats_missing_results_and_sections_as_empty() {
    let config = config(&[]);
    let body = r#"{"web": {}, "news": null, "discussions": {"results": null}}"#;

    let response = answer(&config, "q", &Object::new(), body).unwrap();

    assert!(response.results.is_empty());
}

#[test]
fn brave_search_reply_rejects_a_section_that_is_not_an_object() {
    let config = config(&[]);
    let body = r#"{"web": []}"#;

    let error = answer(&config, "q", &Object::new(), body).unwrap_err();

    assert!(matches!(error, BraveSearchError::ReplyDecode(_)));
}

#[test]
fn brave_search_reply_rejects_a_count_that_is_not_an_integer() {
    let body = r#"{"web": {"results": []}}"#;
    let url = format!("{BRAVE_API_BASE}?count=many");

    let error = super::reply::decode_brave_search(body.as_bytes(), Some(url.as_str()), today())
        .unwrap_err();

    assert!(matches!(error, BraveSearchError::CountNotInteger { .. }));
}

#[test]
fn brave_search_reply_rejects_an_oversized_body() {
    let body = vec![b' '; REPLY_MAX_BYTES.saturating_add(1)];

    let error = super::reply::decode_brave_search(&body, None, today()).unwrap_err();

    assert!(matches!(
        error,
        BraveSearchError::ReplyTooLarge { limit } if limit == REPLY_MAX_BYTES
    ));
}

#[rstest]
#[case::unix_seconds(r#""1700000000""#, Some("2023-11-14"))]
#[case::unix_milliseconds(r#""1700000000000""#, Some("2023-11-14"))]
#[case::above_the_threshold(r#""100000000001""#, Some("1973-03-03"))]
#[case::at_the_threshold(r#""100000000000""#, Some("5138-11-16"))]
#[case::negative_milliseconds(r#""-100000000001""#, Some("1966-10-31"))]
#[case::negative_fraction(r#""-1.5""#, Some("1969-12-31"))]
#[case::negative_zero(r#""-0.0""#, Some("1970-01-01"))]
#[case::string_zero(r#""0""#, Some("1970-01-01"))]
#[case::numeric_zero("0", None)]
#[case::numeric_zero_float("0.0", None)]
#[case::integer_timestamp("1700000000", Some("2023-11-14"))]
#[case::float_timestamp("1700000000000", Some("2023-11-14"))]
#[case::fraction_seconds("1.5", Some("1970-01-01"))]
#[case::negative_integer("-1", Some("1969-12-31"))]
#[case::empty_string(r#""""#, None)]
#[case::null("null", None)]
#[case::true_value("true", None)]
#[case::false_value("false", None)]
#[case::empty_list("[]", None)]
#[case::filled_list("[1]", None)]
#[case::empty_object("{}", None)]
#[case::filled_object(r#"{"a": 1}"#, None)]
fn brave_search_dates_read_json_values(#[case] literal: &str, #[case] expected: Option<&str>) {
    assert_eq!(date_of(literal).as_deref(), expected);
}

#[rstest]
#[case::iso("2024-03-05", Some("2024-03-05"))]
#[case::iso_slash("2024/03/05", Some("2024-03-05"))]
#[case::iso_with_time("2024-3-5T10:00", Some("2024-03-05"))]
#[case::month_first("05/03/2024", Some("2024-05-03"))]
#[case::day_over_twelve("31/12/2024", Some("2024-12-31"))]
#[case::rfc_2822("Tue Mar 05 2024 23:30:00 GMT+0200", Some("2024-03-05"))]
#[case::offset_west("2024-03-05T23:30:00-0500", Some("2024-03-05"))]
#[case::offset_east("2024-03-05T23:30:00+05:00", Some("2024-03-05"))]
#[case::utf_offset("2024-03-05T10:00:00.123456Z", Some("2024-03-05"))]
#[case::weekday_prefix("Thu, 05 Mar 2024 23:30:00 +0000", Some("2024-03-05"))]
#[case::month_name("Mar 5, 2024", Some("2024-03-05"))]
#[case::ordinal("March 5th, 2024 at 10:30 AM", Some("2024-03-05"))]
#[case::month_first_dash("Jan-05-2024", Some("2024-01-05"))]
#[case::year_first_dash("2024-Jan-05", Some("2024-01-05"))]
#[case::day_first_dash("05-Jan-2024", Some("2024-01-05"))]
#[case::month_slash("Jan/05/2024", Some("2024-01-05"))]
#[case::compact_numeric("20240305", Some("1970-08-23"))]
#[case::year_month("2024-03", Some("2024-03-05"))]
#[case::month_year("Mar 2024", Some("2024-03-05"))]
#[case::month_day("Mar 5", Some("2024-03-05"))]
#[case::day_month_year("5 Jan 2024", Some("2024-01-05"))]
#[case::invalid_day("2024-02-30", None)]
#[case::invalid_month("2024-13-05", None)]
#[case::relative_word("yesterday", None)]
#[case::noon_word("noon", None)]
#[case::time_only("10:00", Some("2024-03-05"))]
#[case::time_without_zero_pad("12:30", Some("2024-03-05"))]
#[case::weekday_today("Tuesday", Some("2024-03-05"))]
#[case::weekday_ahead("Sunday", Some("2024-03-10"))]
#[case::weekday_wrapped("Monday", Some("2024-03-11"))]
#[case::weekday_next("Wednesday", Some("2024-03-06"))]
#[case::trailing_meridiem("2024-03-05 10:00:00 PM", Some("2024-03-05"))]
#[case::iso_week_is_not_a_date("2024-W10", None)]
#[case::quarter("Q1 2024", Some("2024-01-05"))]
#[case::relative_days("5 days ago", Some("2024-03-05"))]
#[case::trailing_word("2024-03-05extra", Some("2024-03-05"))]
#[case::leading_word("extra2024-03-05", Some("2024-03-05"))]
#[case::short_year("99-03-05", Some("1999-03-05"))]
#[case::fraction_and_offset("2024-03-05T10:00:00.5-05:00", Some("2024-03-05"))]
#[case::leap_day("2024-02-29T23:59:59.999Z", Some("2024-02-29"))]
fn brave_search_dates_read_text(#[case] value: &str, #[case] expected: Option<&str>) {
    assert_eq!(date_of(&format!("{value:?}")).as_deref(), expected);
}

#[test]
fn brave_search_dates_expose_the_sources_two_flags() {
    let dayfirst = DateFlags {
        dayfirst: true,
        yearfirst: false,
    };
    let value = DateValue::from_literal("\"05/03/2024\"");

    assert_eq!(
        dates::to_yyyy_mm_dd(&value, today(), dayfirst).as_deref(),
        Some("2024-03-05")
    );
    assert_eq!(
        dates::to_yyyy_mm_dd(&value, today(), DateFlags::default()).as_deref(),
        Some("2024-05-03")
    );
}

#[test]
fn brave_search_dates_force_the_year_first_for_iso_ymd() {
    let dayfirst = DateFlags {
        dayfirst: true,
        yearfirst: false,
    };
    let value = DateValue::from_literal("\"2024-03-05\"");

    assert_eq!(
        dates::to_yyyy_mm_dd(&value, today(), dayfirst).as_deref(),
        Some("2024-03-05")
    );
}

#[test]
fn brave_search_dates_clamp_the_default_day_to_the_month() {
    let january_end = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
    let value = DateValue::from_literal("\"2024-02\"");

    assert_eq!(
        dates::to_yyyy_mm_dd(&value, january_end, DateFlags::default()).as_deref(),
        Some("2024-02-29")
    );
}

#[test]
fn brave_search_dates_fill_a_missing_year_from_the_injected_day() {
    let value = DateValue::from_literal("\"Mar 5\"");

    assert_eq!(
        dates::to_yyyy_mm_dd(&value, today(), DateFlags::default()).as_deref(),
        Some("2024-03-05")
    );
}

#[rstest]
fn replays_the_brave_search_corpus(#[files("fuzz/corpus/brave_search/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();

    drop(super::reply::decode_brave_search(&bytes, None, today()));
    if let Ok(text) = std::str::from_utf8(&bytes) {
        drop(date_of(text));
        drop(date_of(&format!("{text:?}")));
    }
}

#[test]
fn brave_search_config_remembers_the_last_url() {
    let config = config(&[]);
    let params = Object::new();

    let url = request_url(&config, "one", &params);
    let body = r#"{"web": {"results": [{"title": "w", "url": "u"}]}}"#;
    let response = config
        .transform_search_response(&ProviderResponse {
            status_code: 200,
            headers: &Headers::new(),
            text: body,
        })
        .unwrap();

    assert_eq!(response.results.len(), 1);
    assert!(url.starts_with(BRAVE_API_BASE));
}
