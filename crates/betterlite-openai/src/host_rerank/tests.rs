#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::jina_infinity_rerank::Settings;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::Value;
use uuid::Uuid;

use super::error::HostRerankError;
use super::{
    HostProvider, JINA_URL, decode_host_rerank, infinity_complete_url, infinity_request_headers,
    jina_complete_url, jina_request_headers, jina_rerank_cost, map_params, transform_request,
};
use crate::api::rerank_contracts::records::RerankBilledUnits;
use crate::api::responses_shapes::wire::ShapeDecodeError;
use crate::cohere_rerank::RerankParams;
use crate::openai_base_config::Headers;

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn params(source: &str) -> RerankParams {
    serde_json::from_str(source).unwrap()
}

fn value(source: &str) -> Value {
    serde_json::from_str(source).unwrap()
}

fn refusal(error: &HostRerankError) -> &'static str {
    match error {
        HostRerankError::MissingApiKey => "missing key",
        HostRerankError::MissingApiBase => "missing base",
        HostRerankError::InvalidApiBase { .. } => "invalid base",
        HostRerankError::Status { .. } => "status",
        HostRerankError::Json { .. } => "json",
        HostRerankError::Decode { .. } => "decode",
        HostRerankError::NoResults => "no results",
        HostRerankError::Document => "document",
        HostRerankError::Counter { .. } => "counter",
        HostRerankError::CounterOverflow => "counter overflow",
        HostRerankError::CostOverflow => "cost overflow",
        HostRerankError::Contract(_) => "contract",
        HostRerankError::Request(_) => "request",
    }
}

fn decode(
    provider: HostProvider,
    status: u16,
    body: &[u8],
) -> (Result<Value, HostRerankError>, Vec<String>) {
    let mut logged = Vec::new();
    let reply = decode_host_rerank(
        provider,
        status,
        body,
        || Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0001),
        |text| logged.push(text.to_owned()),
    );
    let answer = reply.map(|response| {
        let mut serialized = serde_json::to_value(&response).unwrap();
        serialized["hidden"] = serde_json::to_value(&response.hidden_params).unwrap();
        serialized
    });
    (answer, logged)
}

fn at_the_byte_limit() -> Vec<u8> {
    let mut body = b"[]".to_vec();
    body.resize(0x0080_0000, b' ');
    body
}

fn over_the_byte_limit() -> Vec<u8> {
    let limit = 0x0080_0000;
    vec![b' '; limit + 1]
}

fn results_of(count: usize) -> Vec<u8> {
    format!("{{\"results\":[{}]}}", vec!["null"; count].join(",")).into_bytes()
}

#[rstest]
#[case::none(None, Some(JINA_URL))]
#[case::empty(Some(""), Some("/v1/rerank"))]
#[case::bare_host(Some("https://x.test"), Some("https://x.test/v1/rerank"))]
#[case::replaced_path(Some("https://x.test/a/b/"), Some("https://x.test/v1/rerank"))]
#[case::kept_parts(
    Some("http://U:p@X.com:443/a/b?q=1#f"),
    Some("http://U:p@x.com:443/v1/rerank?q=1#f")
)]
#[case::ipv6(Some("http://[::1]:8/a"), Some("http://[::1]:8/v1/rerank"))]
#[case::without_scheme_and_host(Some("localhost:8000"), None)]
#[case::relative(Some("nope"), None)]
fn jina_url_replaces_the_path(#[case] base: Option<&str>, #[case] expected: Option<&str>) {
    let url = jina_complete_url(base);
    match expected {
        Some(url_text) => assert_eq!(url.unwrap(), url_text),
        None => assert!(matches!(url, Err(HostRerankError::InvalidApiBase { .. }))),
    }
}

#[rstest]
#[case::plain("k", "Bearer k")]
#[case::empty("", "Bearer ")]
fn jina_headers_are_lowercase_and_replace_the_caller(#[case] key: &str, #[case] bearer: &str) {
    let headers = jina_request_headers(Some(&secret(key))).unwrap();
    let expected = Headers::from([
        ("accept".to_owned(), "application/json".to_owned()),
        ("content-type".to_owned(), "application/json".to_owned()),
        ("authorization".to_owned(), bearer.to_owned()),
    ]);
    assert_eq!(headers, expected);
    assert!(matches!(
        jina_request_headers(None),
        Err(HostRerankError::MissingApiKey)
    ));
}

#[test]
fn a_missing_key_or_base_keeps_the_litellm_message() {
    assert_eq!(
        HostRerankError::MissingApiKey.to_string(),
        "api_key is required. Set via `api_key` parameter or `JINA_API_KEY` environment variable."
    );
    assert_eq!(
        HostRerankError::MissingApiBase.to_string(),
        "api_base is required for Infinity rerank"
    );
}

#[rstest]
#[case::appended("http://h/v1", "http://h/v1/rerank")]
#[case::slashes_stripped("http://h/v1///", "http://h/v1/rerank")]
#[case::kept("http://h/rerank/", "http://h/rerank")]
#[case::needs_a_segment("http://h/xrerank", "http://h/xrerank/rerank")]
#[case::empty("", "/rerank")]
fn infinity_url_appends_rerank_once(#[case] base: &str, #[case] expected: &str) {
    assert_eq!(infinity_complete_url(Some(base)).unwrap(), expected);
}

#[test]
fn infinity_url_needs_a_base() {
    assert!(matches!(
        infinity_complete_url(None),
        Err(HostRerankError::MissingApiBase)
    ));
}

#[rstest]
#[case::explicit(Some("x"), Some("e"), Some("g"), "Bearer x")]
#[case::environment(None, Some("e"), Some("g"), "Bearer e")]
#[case::global(None, None, Some("g"), "Bearer g")]
#[case::none(None, None, None, "Bearer None")]
fn infinity_key_resolves_in_order(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] global: Option<&str>,
    #[case] expected: &str,
) {
    let settings = Settings {
        infinity_api_key: environment.map(secret),
        infinity_key: global.map(secret),
    };
    let headers =
        infinity_request_headers(&Headers::new(), explicit.map(secret).as_ref(), &settings);
    assert_eq!(headers["Authorization"], expected);
    assert_eq!(headers["accept"], "application/json");
    assert_eq!(headers["content-type"], "application/json");
}

#[test]
fn infinity_caller_headers_overwrite_every_default() {
    let caller = Headers::from([
        ("Authorization".to_owned(), "mine".to_owned()),
        ("accept".to_owned(), "text/plain".to_owned()),
        ("authorization".to_owned(), "low".to_owned()),
        ("x".to_owned(), "1".to_owned()),
    ]);
    let headers = infinity_request_headers(&caller, None, &Settings::default());
    assert_eq!(headers.len(), 5);
    assert_eq!(headers["Authorization"], "mine");
    assert_eq!(headers["accept"], "text/plain");
    assert_eq!(headers["content-type"], "application/json");
}

#[test]
fn jina_options_come_only_from_the_dictionary() {
    let options = params(r#"{"query":"q","top_n":3,"documents":["a"]}"#);
    let dictionary = params(r#"{"top_n":9,"rank_fields":["f"],"return_documents":false}"#);
    let mapped = map_params(HostProvider::Jina, &options, &dictionary);
    assert_eq!(
        serde_json::to_value(mapped).unwrap(),
        value(r#"{"top_n":9,"return_documents":false}"#)
    );
    let empty = map_params(HostProvider::Jina, &options, &RerankParams::new());
    assert!(empty.is_empty());
}

#[test]
fn infinity_keeps_the_cohere_v1_projection() {
    let options = params(r#"{"query":"q","documents":["a"],"max_chunks_per_doc":2}"#);
    let mapped = map_params(HostProvider::Infinity, &options, &RerankParams::new());
    let body = transform_request(HostProvider::Infinity, "m", &mapped).unwrap();
    assert_eq!(
        serde_json::to_value(body).unwrap(),
        value(
            r#"{"model":"m","query":"q","documents":["a"],"max_chunks_per_doc":2,"return_documents":true}"#
        )
    );
    let missing = transform_request(HostProvider::Infinity, "m", &RerankParams::new());
    assert!(matches!(missing, Err(HostRerankError::Request(_))));
}

#[test]
fn jina_request_is_the_model_over_the_options() {
    let body = transform_request(HostProvider::Jina, "m", &params(r#"{"query":"q"}"#)).unwrap();
    assert_eq!(
        serde_json::to_value(body).unwrap(),
        value(r#"{"model":"m","query":"q"}"#)
    );
}

#[rstest]
#[case::priced(Some(r#"{"total_tokens":7}"#), Some("0.25"), "1.75")]
#[case::negative(Some(r#"{"total_tokens":-4}"#), Some("0.25"), "-1")]
#[case::no_price(Some(r#"{"total_tokens":7}"#), None, "0")]
#[case::no_units(None, Some("0.25"), "0")]
#[case::no_total(Some("{}"), Some("0.25"), "0")]
#[case::null_total(Some(r#"{"total_tokens":null}"#), Some("0.25"), "0")]
#[case::search_units_do_not_count(Some(r#"{"search_units":5}"#), Some("0.25"), "0")]
fn jina_cost_is_the_total_tokens_times_the_input_price(
    #[case] units: Option<&str>,
    #[case] price: Option<&str>,
    #[case] expected: &str,
) {
    let billed: Option<RerankBilledUnits> = units.map(|text| serde_json::from_str(text).unwrap());
    let priced = price.map(|text| text.parse::<Decimal>().unwrap());
    let (input, output) = jina_rerank_cost(billed.as_ref(), priced).unwrap();
    assert_eq!(input, expected.parse::<Decimal>().unwrap());
    assert_eq!(output, Decimal::ZERO);
}

#[test]
fn an_unrepresentable_cost_is_refused() {
    let billed: RerankBilledUnits = serde_json::from_str(r#"{"total_tokens":2}"#).unwrap();
    let cost = jina_rerank_cost(Some(&billed), Some(Decimal::MAX));
    assert!(matches!(cost, Err(HostRerankError::CostOverflow)));
}

#[rstest]
#[case::created(201)]
#[case::server_error(500)]
#[case::informational(199)]
fn jina_requires_status_200_before_it_logs(#[case] status: u16) {
    let (reply, logged) = decode(HostProvider::Jina, status, b"{\"results\":[]}");
    assert!(matches!(
        reply,
        Err(HostRerankError::Status { status_code, message })
            if status_code == status && message == "{\"results\":[]}"
    ));
    assert!(logged.is_empty());
}

#[test]
fn jina_logs_the_text_then_fails_on_bad_json() {
    let (reply, logged) = decode(HostProvider::Jina, 200, b"nope");
    assert!(matches!(
        reply,
        Err(HostRerankError::Json {
            status_code: 200,
            ..
        })
    ));
    assert_eq!(logged, ["nope"]);
}

#[rstest]
#[case::copied_usage(
    r#"{"id":"r","usage":{"total_tokens":7,"prompt_tokens":3,"input_tokens":1,"output_tokens":2,"search_units":4},"results":[]}"#,
    r#"{"id":"r","results":[],"meta":{"billed_units":{"search_units":4,"total_tokens":7},"tokens":{"input_tokens":1,"output_tokens":2}},"hidden":{"usage":{"prompt_tokens":3}}}"#
)]
#[case::null_and_absent_usage(
    r#"{"id":"","usage":null,"results":[]}"#,
    r#"{"id":"00000000-0000-4000-8000-000000000001","results":[],"meta":{"billed_units":{},"tokens":{}},"hidden":{}}"#
)]
#[case::null_counter(
    r#"{"usage":{"total_tokens":null},"results":[]}"#,
    r#"{"id":"00000000-0000-4000-8000-000000000001","results":[],"meta":{"billed_units":{"total_tokens":null},"tokens":{}},"hidden":{}}"#
)]
#[case::documents(
    r#"{"id":"r","results":[{"index":1,"relevance_score":0.5,"document":"hi"},{"index":0,"relevance_score":1,"document":{"text":"a","x":1}},{"index":2,"relevance_score":0.25}]}"#,
    r#"{"id":"r","results":[{"index":1,"relevance_score":0.5,"document":{"text":"hi"}},{"index":0,"relevance_score":1,"document":{"text":"a"}},{"index":2,"relevance_score":0.25}],"meta":{"billed_units":{},"tokens":{}},"hidden":{}}"#
)]
fn jina_decodes_its_answer(#[case] body: &str, #[case] expected: &str) {
    let (reply, logged) = decode(HostProvider::Jina, 200, body.as_bytes());
    assert_eq!(reply.unwrap(), value(expected));
    assert_eq!(logged, [body]);
}

#[rstest]
#[case::no_results(r#"{"usage":{}}"#)]
#[case::null_results(r#"{"results":null}"#)]
fn jina_requires_results(#[case] body: &str) {
    let (reply, _) = decode(HostProvider::Jina, 200, body.as_bytes());
    assert!(matches!(reply, Err(HostRerankError::NoResults)));
}

#[rstest]
#[case::null_document(
    r#"{"results":[{"index":0,"relevance_score":1,"document":null}]}"#,
    "contract"
)]
#[case::object_without_text(
    r#"{"results":[{"index":0,"relevance_score":1,"document":{"a":1}}]}"#,
    "document"
)]
#[case::number_document(
    r#"{"results":[{"index":0,"relevance_score":1,"document":5}]}"#,
    "document"
)]
#[case::missing_index(r#"{"results":[{"relevance_score":1}]}"#, "decode")]
#[case::missing_score(r#"{"results":[{"index":0}]}"#, "decode")]
#[case::score_out_of_range(r#"{"results":[{"index":0,"relevance_score":1e999}]}"#, "contract")]
#[case::fractional_counter(r#"{"usage":{"total_tokens":1.5},"results":[]}"#, "counter")]
#[case::counter_beyond_64_bits(
    r#"{"usage":{"total_tokens":9223372036854775808},"results":[]}"#,
    "counter"
)]
#[case::text_counter(r#"{"usage":{"input_tokens":"1"},"results":[]}"#, "counter")]
#[case::not_an_object("[]", "decode")]
fn jina_refuses_shapes_it_cannot_hold(#[case] body: &str, #[case] marker: &str) {
    let (reply, _) = decode(HostProvider::Jina, 200, body.as_bytes());
    let error = reply.expect_err("the shape must be refused");
    assert_eq!(refusal(&error), marker);
}

#[rstest]
#[case::status_is_not_checked(
    500,
    r#"{"id":"i","usage":{"total_tokens":7,"prompt_tokens":3,"completion_tokens":4},"results":[{"index":0,"relevance_score":0.5,"document":"hi"}]}"#,
    r#"{"id":"i","results":[{"index":0,"relevance_score":0.5,"document":{"text":"hi"}}],"meta":{"billed_units":{"total_tokens":7},"tokens":{"input_tokens":3,"output_tokens":4}},"hidden":{"usage":{"completion_tokens":4}}}"#
)]
#[case::missing_everything(
    200,
    "{}",
    r#"{"id":"00000000-0000-4000-8000-000000000001","results":[],"meta":{"billed_units":{},"tokens":{"input_tokens":0,"output_tokens":0}},"hidden":{}}"#
)]
#[case::signed_difference(
    200,
    r#"{"usage":{"total_tokens":1,"prompt_tokens":3},"results":null}"#,
    r#"{"id":"00000000-0000-4000-8000-000000000001","results":[],"meta":{"billed_units":{"total_tokens":1},"tokens":{"input_tokens":3,"output_tokens":-2}},"hidden":{}}"#
)]
#[case::null_prompt_counts_as_absent(
    200,
    r#"{"usage":{"total_tokens":5,"prompt_tokens":null},"results":[]}"#,
    r#"{"id":"00000000-0000-4000-8000-000000000001","results":[],"meta":{"billed_units":{"total_tokens":5},"tokens":{"input_tokens":0,"output_tokens":5}},"hidden":{}}"#
)]
#[case::only_truthy_documents_are_wrapped(
    200,
    r#"{"id":"","results":[{"index":0,"relevance_score":1,"document":""},{"index":1,"relevance_score":1,"document":null},{"index":2,"relevance_score":1,"document":"x"}]}"#,
    r#"{"id":"00000000-0000-4000-8000-000000000001","results":[{"index":0,"relevance_score":1},{"index":1,"relevance_score":1},{"index":2,"relevance_score":1,"document":{"text":"x"}}],"meta":{"billed_units":{},"tokens":{"input_tokens":0,"output_tokens":0}},"hidden":{}}"#
)]
#[case::null_usage_counts_as_absent(
    200,
    r#"{"usage":null,"results":[]}"#,
    r#"{"id":"00000000-0000-4000-8000-000000000001","results":[],"meta":{"billed_units":{},"tokens":{"input_tokens":0,"output_tokens":0}},"hidden":{}}"#
)]
#[case::null_total_tokens_counts_as_absent(
    200,
    r#"{"usage":{"total_tokens":null,"prompt_tokens":5},"results":[]}"#,
    r#"{"id":"00000000-0000-4000-8000-000000000001","results":[],"meta":{"billed_units":{"total_tokens":null},"tokens":{"input_tokens":5,"output_tokens":-5}},"hidden":{}}"#
)]
fn infinity_decodes_its_answer(#[case] status: u16, #[case] body: &str, #[case] expected: &str) {
    let (reply, logged) = decode(HostProvider::Infinity, status, body.as_bytes());
    assert_eq!(reply.unwrap(), value(expected));
    assert!(logged.is_empty());
}

#[rstest]
#[case::not_json(502, "nope")]
#[case::empty(200, "")]
fn infinity_bad_json_carries_the_upstream_status(#[case] status: u16, #[case] sent: &str) {
    let (reply, _) = decode(HostProvider::Infinity, status, sent.as_bytes());
    assert!(matches!(
        reply,
        Err(HostRerankError::Json { status_code, body, .. })
            if status_code == status && body == sent
    ));
}

#[rstest]
#[case::object_document(
    r#"{"results":[{"index":0,"relevance_score":1,"document":{"text":"a"}}]}"#,
    "document"
)]
#[case::falsy_object_document(
    r#"{"results":[{"index":0,"relevance_score":1,"document":{}}]}"#,
    "document"
)]
#[case::missing_index(r#"{"results":[{"relevance_score":1}]}"#, "decode")]
#[case::counter_beyond_64_bits(r#"{"usage":{"prompt_tokens":9223372036854775808}}"#, "counter")]
#[case::overflowing_difference(
    r#"{"usage":{"total_tokens":-9223372036854775808,"prompt_tokens":1}}"#,
    "counter overflow"
)]
#[case::not_an_object("[]", "decode")]
fn infinity_refuses_shapes_it_cannot_hold(#[case] body: &str, #[case] marker: &str) {
    let (reply, _) = decode(HostProvider::Infinity, 200, body.as_bytes());
    let error = reply.expect_err("the shape must be refused");
    assert_eq!(refusal(&error), marker);
}

#[rstest]
#[case::jina(HostProvider::Jina)]
#[case::infinity(HostProvider::Infinity)]
fn a_wrong_shape_is_a_decode_error_and_not_a_json_failure(#[case] provider: HostProvider) {
    let (wrong, _) = decode(provider, 200, b"[]");
    assert!(matches!(
        wrong,
        Err(HostRerankError::Decode {
            source: ShapeDecodeError::Json(_)
        })
    ));
    let (syntax, _) = decode(provider, 200, b"{");
    assert!(matches!(syntax, Err(HostRerankError::Json { .. })));
}

fn nested_body(depth: usize) -> Vec<u8> {
    let mut body = "[".repeat(depth);
    body.push_str(&"]".repeat(depth));
    body.into_bytes()
}

fn over_limit(body: &[u8]) -> bool {
    let (reply, _) = decode(HostProvider::Infinity, 200, body);
    matches!(
        reply,
        Err(HostRerankError::Decode {
            source: ShapeDecodeError::JsonTooLarge
                | ShapeDecodeError::TooManyItems
                | ShapeDecodeError::TooDeep
        })
    )
}

#[rstest]
#[case::bytes(over_the_byte_limit(), true)]
#[case::bytes_at_the_limit(at_the_byte_limit(), false)]
#[case::items_at_the_limit(results_of(199_999), false)]
#[case::items(results_of(200_001), true)]
#[case::nesting_at_the_limit(nested_body(129), false)]
#[case::nesting(nested_body(130), true)]
fn the_decoder_limits_hold(#[case] body: Vec<u8>, #[case] expected: bool) {
    assert_eq!(over_limit(&body), expected);
}

#[test]
fn bytes_that_are_not_utf8_are_refused_with_their_length() {
    let (infinity, infinity_logged) = decode(HostProvider::Infinity, 200, &[0xff, 0xfe]);
    let infinity_error = infinity.expect_err("bytes that are not UTF-8 are refused");
    assert!(matches!(
        infinity_error,
        HostRerankError::Json {
            status_code: 200,
            ..
        }
    ));
    assert_eq!(infinity_error.to_string(), "2 bytes that are not UTF-8");
    assert!(infinity_logged.is_empty());
    let (jina, jina_logged) = decode(HostProvider::Jina, 200, &[0xff, 0xfe]);
    let jina_error = jina.expect_err("Jina refuses the same bytes");
    assert!(matches!(jina_error, HostRerankError::Json { .. }));
    assert_eq!(jina_logged, ["2 bytes that are not UTF-8"]);
    let (refused, refused_logged) = decode(HostProvider::Jina, 502, &[0xff]);
    let status_error = refused.expect_err("the status is refused before the text is logged");
    assert!(matches!(
        status_error,
        HostRerankError::Status {
            status_code: 502,
            ..
        }
    ));
    assert_eq!(status_error.to_string(), "1 bytes that are not UTF-8");
    assert!(refused_logged.is_empty());
}

#[rstest]
fn replays_the_host_rerank_corpus(#[files("fuzz/corpus/host_rerank/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    for provider in [HostProvider::Jina, HostProvider::Infinity] {
        let _reply = decode_host_rerank(provider, 200, &bytes, Uuid::nil, |_| {});
    }
}
