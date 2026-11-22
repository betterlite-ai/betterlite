#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fs;
use std::mem;
use std::path::PathBuf;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::CredentialError;
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, ProviderResponse, RequestInput, SearchProvider as _, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use rstest::rstest;
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::error::TinyfishSearchError;
use super::reply::{REPLY_MAX_BYTES, decode_tinyfish_search};
use super::request::{self, CallerLimit, TinyfishBody, TinyfishParam, limit_of_float};
use super::{TINYFISH_API_BASE, TINYFISH_RESULT_CAP, TinyfishSearch};

fn environment(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn params(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

fn single(query: &str) -> SearchQuery {
    SearchQuery::Single(query.to_owned())
}

fn body_of(
    search: &TinyfishSearch<'_>,
    query: &SearchQuery,
    optional_params: &Object,
) -> TinyfishBody {
    let headers = Headers::new();
    search
        .transform_search_request(&RequestInput {
            query,
            optional_params,
            api_key: None,
            api_base: None,
            headers: &headers,
        })
        .unwrap()
}

fn parsed(
    search: &TinyfishSearch<'_>,
    status_code: u16,
    text: &str,
) -> Result<SearchResponse, TinyfishSearchError> {
    let headers = Headers::new();
    search.transform_search_response(&ProviderResponse {
        status_code,
        headers: &headers,
        text,
    })
}

fn reply_with_results(count: usize) -> String {
    let items: Vec<String> = (0..count)
        .map(|index| {
            format!(r#"{{"title":"t{index}","url":"https://example.test/{index}","snippet":"s"}}"#)
        })
        .collect();
    format!(r#"{{"results":[{}]}}"#, items.join(","))
}

fn limit_of(optional_params: &Object) -> CallerLimit {
    let (_, limit) = request::build(&single("q"), optional_params);
    limit
}

fn hidden_params_for(headers: &Headers) -> Object {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let reply = search
        .transform_search_response(&ProviderResponse {
            status_code: 200,
            headers,
            text: r#"{"results":[]}"#,
        })
        .unwrap();
    reply.hidden_params
}

#[test]
fn tinyfish_reports_its_display_name_and_get_method() {
    assert_eq!(TinyfishSearch::ui_friendly_name(), "TinyFish");
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    assert_eq!(search.http_method(), HttpMethod::Get);
    assert!(!search.supports_rich_search_input());
}

#[test]
fn tinyfish_search_settings_default_to_an_unset_key() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let error = search
        .validated_headers(Headers::new(), None, None)
        .unwrap_err();
    assert!(matches!(error, TinyfishSearchError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "TINYFISH_API_KEY is not set. Set `TINYFISH_API_KEY` environment variable."
    );
}

#[test]
fn tinyfish_search_settings_read_the_environment() {
    let environment = environment(&[
        ("TINYFISH_API_KEY", "env-key"),
        ("TINYFISH_API_BASE", "https://proxy.test/v1"),
    ]);
    let search = TinyfishSearch::load(&environment);
    let headers = search
        .validated_headers(Headers::new(), None, None)
        .unwrap();
    assert_eq!(
        headers.get("X-API-Key").map(String::as_str),
        Some("env-key")
    );
    assert_eq!(
        headers.get("Accept").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(headers.get("Content-Type"), None);
    let url = search
        .complete_url(None, &Object::new(), None, None)
        .unwrap();
    assert_eq!(url, "https://proxy.test/v1");
}

#[test]
fn tinyfish_search_settings_ignore_empty_values() {
    let environment = environment(&[("TINYFISH_API_KEY", ""), ("TINYFISH_API_BASE", "")]);
    let search = TinyfishSearch::load(&environment);
    let error = search
        .validated_headers(Headers::new(), None, None)
        .unwrap_err();
    assert!(matches!(error, TinyfishSearchError::MissingApiKey));
    let url = search
        .complete_url(None, &Object::new(), None, None)
        .unwrap();
    assert_eq!(url, "https://api.search.tinyfish.ai");
}

#[test]
fn tinyfish_search_settings_accept_the_caller_key_on_any_base() {
    let environment = environment(&[("TINYFISH_API_KEY", "env-key")]);
    let search = TinyfishSearch::load(&environment);
    let caller = SecretString::from("caller-key".to_owned());
    let headers = search
        .validated_headers(
            Headers::new(),
            Some(&caller),
            Some("https://elsewhere.test"),
        )
        .unwrap();
    assert_eq!(
        headers.get("X-API-Key").map(String::as_str),
        Some("caller-key")
    );
}

#[test]
fn tinyfish_search_settings_trust_the_default_and_environment_base() {
    let environment = environment(&[
        ("TINYFISH_API_KEY", "env-key"),
        ("TINYFISH_API_BASE", "https://proxy.test/v1"),
    ]);
    let search = TinyfishSearch::load(&environment);
    let default_base = search
        .validated_headers(Headers::new(), None, Some(TINYFISH_API_BASE))
        .unwrap();
    assert_eq!(
        default_base.get("X-API-Key").map(String::as_str),
        Some("env-key")
    );
    let environment_base = search
        .validated_headers(Headers::new(), None, Some("https://proxy.test/v1"))
        .unwrap();
    assert_eq!(
        environment_base.get("X-API-Key").map(String::as_str),
        Some("env-key")
    );
}

#[test]
fn tinyfish_search_settings_refuse_the_server_key_for_an_untrusted_base() {
    let environment = environment(&[("TINYFISH_API_KEY", "env-key")]);
    let search = TinyfishSearch::load(&environment);
    let error = search
        .validated_headers(Headers::new(), None, Some("https://evil.test"))
        .unwrap_err();
    assert!(matches!(
        error,
        TinyfishSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
    let body = body_of(&search, &single("q"), &Object::new());
    let error = search
        .sign_request(
            Headers::new(),
            &OutboundSearch {
                api_base: "https://evil.test",
                api_key: None,
                optional_params: &Object::new(),
                body: &body,
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        TinyfishSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
}

#[test]
fn tinyfish_validate_environment_keeps_the_incoming_headers() {
    let environment = environment(&[("TINYFISH_API_KEY", "env-key")]);
    let search = TinyfishSearch::load(&environment);
    let mut incoming = Headers::new();
    incoming.insert("X-Trace".to_owned(), "abc".to_owned());
    let validated = search.validate_environment(incoming, None, None);
    assert_eq!(validated.get("X-Trace").map(String::as_str), Some("abc"));
    assert_eq!(
        validated.get("X-API-Key").map(String::as_str),
        Some("env-key")
    );
    assert_eq!(
        validated.get("Accept").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(validated.get("Content-Type"), None);
}

#[test]
fn tinyfish_validate_environment_stays_infallible_on_a_missing_key() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let validated = search.validate_environment(Headers::new(), None, None);
    assert_eq!(validated.get("X-API-Key"), None);
}

#[test]
fn tinyfish_sign_request_adds_the_headers_and_no_body() {
    let environment = environment(&[("TINYFISH_API_KEY", "env-key")]);
    let search = TinyfishSearch::load(&environment);
    let body = body_of(&search, &single("q"), &Object::new());
    let (headers, signed_body) = search
        .sign_request(
            Headers::new(),
            &OutboundSearch {
                api_base: TINYFISH_API_BASE,
                api_key: None,
                optional_params: &Object::new(),
                body: &body,
            },
        )
        .unwrap();
    assert_eq!(
        headers.get("X-API-Key").map(String::as_str),
        Some("env-key")
    );
    assert_eq!(
        headers.get("Accept").map(String::as_str),
        Some("application/json")
    );
    assert!(signed_body.is_none());
}

#[test]
fn tinyfish_record_joins_a_query_list_with_spaces() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let body = body_of(
        &search,
        &SearchQuery::Multiple(vec!["rust".to_owned(), "async".to_owned()]),
        &Object::new(),
    );
    assert_eq!(
        body.params.get("query"),
        Some(&TinyfishParam::Text("rust async".to_owned()))
    );
    assert_eq!(
        serde_json::to_value(&body).unwrap(),
        serde_json::from_str::<JsonValue>(r#"{"_tinyfish_params":{"query":"rust async"}}"#)
            .unwrap()
    );
}

#[test]
fn tinyfish_record_rewrites_the_domain_filter_into_the_query() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let optional_params = params(r#"{"search_domain_filter":["a.com","b.com"]}"#);
    let body = body_of(&search, &single("rust async"), &optional_params);
    assert_eq!(
        body.params.get("query"),
        Some(&TinyfishParam::Text(
            "(rust async) (site:a.com OR site:b.com)".to_owned()
        ))
    );
    assert_eq!(body.params.get("search_domain_filter"), None);
}

#[rstest]
#[case::text(r#"{"search_domain_filter":"a.com"}"#)]
#[case::number_in_list(r#"{"search_domain_filter":["a.com",3]}"#)]
#[case::null(r#"{"search_domain_filter":null}"#)]
#[case::empty(r#"{"search_domain_filter":[]}"#)]
fn tinyfish_record_ignores_a_domain_filter_that_is_not_a_string_list(#[case] optional: &str) {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let optional_params = params(optional);
    let body = body_of(&search, &single("q"), &optional_params);
    assert_eq!(
        body.params.get("query"),
        Some(&TinyfishParam::Text("q".to_owned()))
    );
    assert_eq!(body.params.get("search_domain_filter"), None);
}

#[test]
fn tinyfish_record_maps_country_to_location() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let optional_params = params(r#"{"country":"US","location":"Berlin"}"#);
    let body = body_of(&search, &single("q"), &optional_params);
    assert_eq!(
        body.params.get("location"),
        Some(&TinyfishParam::Text("US".to_owned()))
    );
    assert_eq!(body.params.get("country"), None);
}

#[test]
fn tinyfish_record_drops_a_non_string_country() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let optional_params = params(r#"{"country":5}"#);
    let body = body_of(&search, &single("q"), &optional_params);
    assert_eq!(body.params.get("country"), None);
    assert_eq!(body.params.get("location"), None);
}

#[test]
fn tinyfish_record_keeps_the_search_query_over_a_caller_option() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let optional_params = params(r#"{"query":"caller","location":"Berlin"}"#);
    let body = body_of(&search, &single("q"), &optional_params);
    assert_eq!(
        body.params.get("query"),
        Some(&TinyfishParam::Text("q".to_owned()))
    );
    assert_eq!(
        body.params.get("location"),
        Some(&TinyfishParam::Text("Berlin".to_owned()))
    );
}

#[test]
fn tinyfish_record_encodes_structured_passthrough_values() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let optional_params = params(
        r#"{"count":7,"filter":{"a":1,"b":[true,null]},"max_tokens_per_page":5,"ratio":2.5,"safe":true,"tags":["x","y"],"text":"T","unsafe":false}"#,
    );
    let body = body_of(&search, &single("q"), &optional_params);
    let expected = serde_json::from_str::<JsonValue>(
        r#"{"_tinyfish_params":{"count":7,"filter":"{\"a\":1,\"b\":[true,null]}","query":"q","ratio":2.5,"safe":"true","tags":"[\"x\",\"y\"]","text":"T","unsafe":"false"}}"#,
    )
    .unwrap();
    assert_eq!(serde_json::to_value(&body).unwrap(), expected);
}

#[rstest]
#[case::zero("0", Some(1))]
#[case::negative("-3", Some(1))]
#[case::one("1", Some(1))]
#[case::five("5", Some(5))]
#[case::ten("10", Some(10))]
#[case::above("50", Some(10))]
#[case::float("2.9", Some(2))]
#[case::negative_float("-2.9", Some(1))]
#[case::float_above("10.9", Some(10))]
#[case::large_float("50.5", Some(10))]
#[case::whole_float("1.0", Some(1))]
#[case::exponent("1e2", Some(10))]
#[case::text("The string \"7\"", Some(7))]
#[case::negative_text("The string \"-2\"", Some(1))]
#[case::padded_text("The string \" 7 \"", Some(7))]
#[case::signed_text("The string \"+7\"", Some(7))]
#[case::underscored_text("The string \"1_0\"", Some(10))]
#[case::zero_text("The string \"0\"", Some(1))]
#[case::huge_integer("123456789012345678901234567890", Some(10))]
#[case::huge_negative_integer("-123456789012345678901234567890", Some(1))]
#[case::true_value("true", Some(1))]
#[case::false_value("false", Some(1))]
#[case::invalid_text("The string \"abc\"", None)]
#[case::fraction_text("The string \"2.5\"", None)]
#[case::double_underscore("The string \"1__0\"", None)]
#[case::trailing_underscore("The string \"1_\"", None)]
#[case::leading_underscore("The string \"_1\"", None)]
#[case::hex_text("The string \"0x10\"", None)]
#[case::empty_text("The string \"\"", None)]
#[case::blank_text("The string \"  \"", None)]
#[case::exponent_text("The string \"1e3\"", None)]
fn tinyfish_max_results_clamps_into_the_one_to_ten_range(
    #[case] raw: &str,
    #[case] expected: Option<usize>,
) {
    let optional_params = match raw.strip_prefix("The string ") {
        Some(text) => params(&format!(r#"{{"max_results":{text}}}"#)),
        None => params(&format!(r#"{{"max_results":{raw}}}"#)),
    };
    let limit = limit_of(&optional_params);
    match expected {
        Some(value) => assert_eq!(limit, CallerLimit::Set(value)),
        None => assert!(matches!(limit, CallerLimit::Ignored(_))),
    }
}

#[rstest]
#[case::null("null")]
#[case::list("[]")]
#[case::object("{}")]
fn tinyfish_max_results_ignores_a_non_scalar_value(#[case] raw: &str) {
    let optional_params = params(&format!(r#"{{"max_results":{raw}}}"#));
    assert_eq!(limit_of(&optional_params), CallerLimit::Absent);
}

#[test]
fn tinyfish_max_results_rejects_non_finite_floats() {
    assert_eq!(
        limit_of_float(Some(f64::INFINITY), "inf"),
        CallerLimit::Ignored("inf".to_owned())
    );
    assert_eq!(
        limit_of_float(Some(f64::NEG_INFINITY), "-inf"),
        CallerLimit::Ignored("-inf".to_owned())
    );
    assert_eq!(
        limit_of_float(Some(f64::NAN), "nan"),
        CallerLimit::Ignored("nan".to_owned())
    );
    assert_eq!(
        limit_of_float(None, "1e400"),
        CallerLimit::Ignored("1e400".to_owned())
    );
    assert_eq!(limit_of_float(Some(2.9), "2.9"), CallerLimit::Set(2));
    assert_eq!(limit_of_float(Some(0.5), "0.5"), CallerLimit::Set(1));
    assert_eq!(limit_of_float(Some(10.9), "10.9"), CallerLimit::Set(10));
}

#[test]
fn tinyfish_max_results_warns_and_keeps_the_stored_limit() {
    let environment = environment(&[("TINYFISH_API_KEY", "key")]);
    let search = TinyfishSearch::load(&environment);
    let good = params(r#"{"max_results":3}"#);
    body_of(&search, &single("q"), &good);
    let invalid = params(r#"{"max_results":"abc"}"#);
    let (_, warnings) = capture(|| body_of(&search, &single("q"), &invalid));
    assert_eq!(
        warnings,
        vec!["TinyFish Search: max_results='abc' is not a valid integer; ignoring.".to_owned()]
    );
    let reply = parsed(&search, 200, &reply_with_results(5)).unwrap();
    assert_eq!(reply.results.len(), 3);
}

#[test]
fn tinyfish_max_results_keeps_the_stored_limit_when_absent() {
    let environment = environment(&[("TINYFISH_API_KEY", "key")]);
    let search = TinyfishSearch::load(&environment);
    let good = params(r#"{"max_results":2}"#);
    body_of(&search, &single("q"), &good);
    body_of(&search, &single("q"), &Object::new());
    let reply = parsed(&search, 200, &reply_with_results(5)).unwrap();
    assert_eq!(reply.results.len(), 2);
}

#[test]
fn tinyfish_reply_cuts_results_to_the_caller_limit_or_ten() {
    let environment = environment(&[("TINYFISH_API_KEY", "key")]);
    let search = TinyfishSearch::load(&environment);
    let text = reply_with_results(12);
    let reply = parsed(&search, 200, &text).unwrap();
    assert_eq!(reply.results.len(), TINYFISH_RESULT_CAP);
    let optional_params = params(r#"{"max_results":4}"#);
    body_of(&search, &single("q"), &optional_params);
    assert_eq!(parsed(&search, 200, &text).unwrap().results.len(), 4);
    assert_eq!(
        parsed(&search, 200, &reply_with_results(3))
            .unwrap()
            .results
            .len(),
        3
    );
}

#[test]
fn tinyfish_complete_url_returns_the_base_without_a_record() {
    let stripped = environment(&[("TINYFISH_API_KEY", "key"), ("TINYFISH_API_BASE", "")]);
    let unset_base = TinyfishSearch::load(&stripped);
    assert_eq!(
        unset_base
            .complete_url(None, &Object::new(), None, None)
            .unwrap(),
        "https://api.search.tinyfish.ai"
    );
    let environment = environment(&[("TINYFISH_API_KEY", "key")]);
    let search = TinyfishSearch::load(&environment);
    assert_eq!(
        search
            .complete_url(Some("https://caller.test/v1"), &Object::new(), None, None)
            .unwrap(),
        "https://caller.test/v1"
    );
    assert_eq!(
        search
            .complete_url(Some(""), &Object::new(), None, None)
            .unwrap(),
        "https://api.search.tinyfish.ai"
    );
    assert_eq!(
        search
            .complete_url(None, &Object::new(), None, None)
            .unwrap(),
        "https://api.search.tinyfish.ai"
    );
}

#[test]
fn tinyfish_complete_url_appends_the_query_string() {
    let environment = environment(&[("TINYFISH_API_KEY", "key")]);
    let search = TinyfishSearch::load(&environment);
    let optional_params = params(r#"{"phrase":"a&b c","suffix":"tail"}"#);
    let body = body_of(&search, &single("q"), &optional_params);
    let url = search
        .complete_url(None, &optional_params, Some(&body), None)
        .unwrap();
    assert_eq!(
        url,
        "https://api.search.tinyfish.ai?query=q&phrase=a%26b+c&suffix=tail"
    );
}

#[test]
fn tinyfish_complete_url_encodes_numbers_like_python() {
    let environment = environment(&[("TINYFISH_API_KEY", "key")]);
    let search = TinyfishSearch::load(&environment);
    let optional_params = params(r#"{"count":7,"ratio":2.5,"whole":10.0}"#);
    let body = body_of(&search, &single("q"), &optional_params);
    let url = search
        .complete_url(None, &optional_params, Some(&body), None)
        .unwrap();
    assert_eq!(
        url,
        "https://api.search.tinyfish.ai?query=q&count=7&ratio=2.5&whole=10.0"
    );
}

#[test]
fn tinyfish_complete_url_rejects_a_null_parameter() {
    let environment = environment(&[("TINYFISH_API_KEY", "key")]);
    let search = TinyfishSearch::load(&environment);
    let optional_params = params(r#"{"session":null}"#);
    let body = body_of(&search, &single("q"), &optional_params);
    assert_eq!(body.params.get("session"), Some(&TinyfishParam::Null));
    let error = search
        .complete_url(None, &optional_params, Some(&body), None)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the TinyFish search parameter `session` is null, and a query parameter must be a string, number or bool"
    );
}

#[test]
fn tinyfish_reply_wraps_a_non_success_status() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let mut headers = Headers::new();
    headers.insert("x-request-id".to_owned(), "req-1".to_owned());
    let error = search
        .transform_search_response(&ProviderResponse {
            status_code: 429,
            headers: &headers,
            text: r#"{"error":{"message":"slow down"}}"#,
        })
        .unwrap_err();
    let TinyfishSearchError::Provider(provider) = error else {
        panic!("expected the wrapped provider error");
    };
    assert_eq!(
        provider.message,
        "TinyFish Search: slow down. See https://docs.tinyfish.ai/search-api for details."
    );
    assert_eq!(provider.status_code, 429);
    assert_eq!(
        provider.headers,
        Some(vec![("x-request-id".to_owned(), "req-1".to_owned())])
    );
}

#[rstest]
#[case::plain(
    "plain failure",
    "TinyFish Search: plain failure. See https://docs.tinyfish.ai/search-api for details."
)]
#[case::nested_json(
    r#"{"error":{"code":"x","message":""}}"#,
    r#"TinyFish Search: {"error":{"code":"x","message":""}}. See https://docs.tinyfish.ai/search-api for details."#
)]
#[case::numeric_message(
    r#"{"error":{"message":7}}"#,
    r#"TinyFish Search: {"error":{"message":7}}. See https://docs.tinyfish.ai/search-api for details."#
)]
#[case::error_not_an_object(
    r#"{"error":"nope"}"#,
    r#"TinyFish Search: {"error":"nope"}. See https://docs.tinyfish.ai/search-api for details."#
)]
fn tinyfish_reply_falls_back_to_the_raw_error_text(#[case] text: &str, #[case] message: &str) {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let error = parsed(&search, 400, text).unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[test]
fn tinyfish_http_error_wraps_the_status_and_body() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let headers = Headers::new();
    let error = search.http_error(&ProviderResponse {
        status_code: 500,
        headers: &headers,
        text: r#"{"error":{"message":"boom"}}"#,
    });
    assert_eq!(
        error.message,
        "TinyFish Search: boom. See https://docs.tinyfish.ai/search-api for details."
    );
    assert_eq!(error.status_code, 500);
}

#[test]
fn tinyfish_error_class_stays_the_base_one() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let headers = Headers::new();
    let error = search.error_class("network down", 502, &headers);
    assert_eq!(error.message, "network down");
    assert_eq!(error.status_code, 502);
}

#[test]
fn tinyfish_reply_reports_a_non_json_body_with_the_first_two_hundred_characters() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let text = format!("{}tail", "x".repeat(220));
    let error = parsed(&search, 200, &text).unwrap_err();
    let expected = format!(
        "TinyFish Search: Expected JSON response, got: {}. See https://docs.tinyfish.ai/search-api for details.",
        "x".repeat(200)
    );
    assert_eq!(error.to_string(), expected);
}

#[rstest]
#[case::missing_results(r#"{"object":"search"}"#)]
#[case::results_not_a_list(r#"{"results":{"title":"x"}}"#)]
#[case::item_not_an_object(r#"{"results":["x"]}"#)]
#[case::object_not_text(r#"{"results":[],"object":7}"#)]
#[case::date_not_text(r#"{"results":[{"title":"t","url":"u","snippet":"s","date":7}]}"#)]
fn tinyfish_reply_rejects_a_body_outside_the_search_response_schema(#[case] text: &str) {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let error = parsed(&search, 200, text).unwrap_err();
    assert!(error.to_string().starts_with(
        "TinyFish Search: Response shape does not match LiteLLM's SearchResponse schema: "
    ));
    assert!(
        error
            .to_string()
            .ends_with(". See https://docs.tinyfish.ai/search-api for details.")
    );
}

#[rstest]
#[case::ok(200, true)]
#[case::last_success(299, true)]
#[case::redirect(300, false)]
#[case::informational(199, false)]
fn tinyfish_reply_treats_only_two_hundreds_as_success(
    #[case] status_code: u16,
    #[case] success: bool,
) {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let reply = parsed(&search, status_code, r#"{"results":[]}"#);
    if success {
        reply.unwrap();
    } else {
        reply.unwrap_err();
    }
}

#[test]
fn tinyfish_reply_defaults_missing_and_non_string_result_fields() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let text = r#"{"results":[{"title":7,"url":null,"extra_field":true},{"title":"t","url":"u","snippet":"s","date":"2024-01-01","last_updated":"2024-01-02"}],"object":"custom","query":"q","total_results":2}"#;
    let reply = parsed(&search, 200, text).unwrap();
    let degraded = &reply.results[0];
    assert_eq!(degraded.title, "");
    assert_eq!(degraded.url, "");
    assert_eq!(degraded.snippet, "");
    assert_eq!(
        degraded.extra.get("extra_field"),
        Some(&JsonValue::Bool(true))
    );
    let complete = &reply.results[1];
    assert_eq!(complete.title, "t");
    assert_eq!(complete.date.as_deref(), Some("2024-01-01"));
    assert_eq!(complete.last_updated.as_deref(), Some("2024-01-02"));
    assert_eq!(reply.object.0, "custom");
    assert_eq!(
        reply.extra.get("query").and_then(JsonValue::as_str),
        Some("q")
    );
    assert_eq!(
        reply.extra.get("total_results").and_then(JsonValue::as_i64),
        Some(2)
    );
}

#[test]
fn tinyfish_reply_defaults_the_object_name() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let reply = parsed(&search, 200, r#"{"results":[]}"#).unwrap();
    assert_eq!(reply.object.0, "search");
}

#[test]
fn tinyfish_reply_logs_well_formed_parameter_warnings() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let text = r#"{"results":[],"parameter_warnings":[
        {"type":"ignored","parameter":"site","message":"ignored site"},
        {"type":"","parameter":"x","message":"m"},
        {"type":"t","parameter":"","message":"m"},
        {"type":"t","parameter":"p","message":""},
        {"type":7,"parameter":"p","message":"m"},
        {"type":"t","parameter":"p"},
        "not an object",
        {"type":"clamped","parameter":"count","message":"clamped count"}]}"#;
    let (reply, warnings) = capture(|| parsed(&search, 200, text).unwrap());
    assert_eq!(
        warnings,
        vec![
            "TinyFish Search parameter_warning (ignored) `site`: ignored site".to_owned(),
            "TinyFish Search parameter_warning (clamped) `count`: clamped count".to_owned(),
        ]
    );
    assert!(reply.extra.contains_key("parameter_warnings"));
}

#[test]
fn tinyfish_reply_keeps_the_raw_and_sanitized_headers() {
    let mut headers = Headers::new();
    headers.insert("x-request-id".to_owned(), "req-1".to_owned());
    headers.insert("x-ratelimit-remaining-requests".to_owned(), "9".to_owned());
    headers.insert("content-type".to_owned(), "application/json".to_owned());
    let hidden = hidden_params_for(&headers);
    let raw = hidden.get("headers").unwrap();
    assert_eq!(
        raw.get("x-request-id").and_then(JsonValue::as_str),
        Some("req-1")
    );
    assert_eq!(
        raw.get("content-type").and_then(JsonValue::as_str),
        Some("application/json")
    );
    let additional = hidden.get("additional_headers").unwrap();
    assert_eq!(
        additional
            .get("llm_provider-x-request-id")
            .and_then(JsonValue::as_str),
        Some("req-1")
    );
    assert_eq!(
        additional
            .get("x-ratelimit-remaining-requests")
            .and_then(JsonValue::as_str),
        Some("9")
    );
    assert_eq!(
        additional
            .get("llm_provider-x-ratelimit-remaining-requests")
            .and_then(JsonValue::as_str),
        Some("9")
    );
}

#[test]
fn tinyfish_reply_rejects_a_body_over_the_reply_bound() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let text = " ".repeat(REPLY_MAX_BYTES + 1);
    let error = parsed(&search, 200, &text).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("the TinyFish search reply is larger than {REPLY_MAX_BYTES} bytes")
    );
}

#[test]
fn tinyfish_reply_accepts_a_body_exactly_at_the_reply_bound() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let text = " ".repeat(REPLY_MAX_BYTES);
    let error = parsed(&search, 200, &text).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("TinyFish Search: Expected JSON response, got: ")
    );
}

#[test]
fn tinyfish_reply_checks_the_status_before_the_body() {
    let environment = environment(&[]);
    let search = TinyfishSearch::load(&environment);
    let text = " ".repeat(REPLY_MAX_BYTES + 1);
    let error = parsed(&search, 500, &text).unwrap_err();
    assert!(matches!(error, TinyfishSearchError::Provider(_)));
}

#[rstest]
fn replays_the_tinyfish_corpus(#[files("fuzz/corpus/tinyfish_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = from_utf8(&bytes) else {
        return;
    };
    let headers = Headers::new();
    if let Ok(reply) = decode_tinyfish_search(text, 200, &headers, TINYFISH_RESULT_CAP) {
        assert_eq!(reply.object.0, "search");
        assert!(reply.results.len() <= TINYFISH_RESULT_CAP);
    }
}

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<String>>>);

impl CapturedLogs {
    fn take(&self) -> Vec<String> {
        match self.0.lock() {
            Ok(mut messages) => mem::take(&mut messages),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

fn capture<T, F>(call: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let logs = CapturedLogs::default();
    let subscriber = registry().with(logs.clone());
    let value = with_default(subscriber, call);
    (value, logs.take())
}
