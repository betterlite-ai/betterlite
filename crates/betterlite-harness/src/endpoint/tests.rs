#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use betterlite_core::cost::call_price::error::CallPriceError;
use betterlite_models::price_lookup::PriceLookup as _;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use http::header::{AUTHORIZATION, HeaderValue};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};

use super::catalog_prices::CatalogPrices;
use super::clock::{Clock, SystemClock};
use super::cost::{CostError, CostPerToken, compute_cost, header_cost, hidden_cost};
use super::encode::{Chunk, EncodeError, encode_chunk, stream_trailer};
use super::errors::Failure;
use super::headers::{extract_token, gateway_headers, response_headers};
use super::random::{RandomBytes, SystemRandom};
use super::redact::sanitize;
use super::route::{POST_ROUTES, ROUTE_PREFIXES, Route, route_of};
use super::sse::SSEUsageParser;
use super::usage::{TokenCounts, UsageTracker, usage_from_body, usage_from_mapping};
use super::{EndpointError, EndpointSetup, HARNESS_SESSION_TOKEN_BYTES, ModelEndpoint};
use crate::context::GatewayTarget;
use crate::types::{Harness, JsonObject, Usage};

fn dec(text: &str) -> Decimal {
    Decimal::from_str_exact(text).unwrap()
}

fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(*name, HeaderValue::from_str(value).unwrap());
    }
    map
}

fn gateway() -> GatewayTarget {
    GatewayTarget {
        api_base: "http://127.0.0.1:4000".to_owned(),
        api_key: SecretString::from("sk-gateway".to_owned()),
    }
}

fn header_text(headers: &HeaderMap, name: &str) -> String {
    headers.get(name).unwrap().to_str().unwrap().to_owned()
}

struct FakePrices {
    per_token: Decimal,
    fail: bool,
}

struct FixedPrices(Decimal);

struct FixedRandom(u8);

impl RandomBytes for FixedRandom {
    fn fill(&self, buffer: &mut [u8]) -> Result<(), getrandom::Error> {
        buffer.fill(self.0);
        Ok(())
    }
}

struct FailingRandom;

impl RandomBytes for FailingRandom {
    fn fill(&self, _buffer: &mut [u8]) -> Result<(), getrandom::Error> {
        Err(getrandom::Error::UNSUPPORTED)
    }
}

struct FixedClock(DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

fn fixed_clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock(DateTime::from_timestamp(0, 0).unwrap()))
}

impl CostPerToken for FixedPrices {
    fn cost_per_token(
        &self,
        _model: &str,
        _input_tokens: u64,
        _output_tokens: u64,
    ) -> Result<Decimal, CostError> {
        Ok(self.0)
    }
}

impl CostPerToken for FakePrices {
    fn cost_per_token(
        &self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<Decimal, CostError> {
        if self.fail {
            return Err(CostError::Unpriced {
                model: model.to_owned(),
                source: Box::new(CallPriceError::NullModel),
            });
        }
        let units = input_tokens.saturating_add(output_tokens);
        Decimal::from(units)
            .checked_mul(self.per_token)
            .ok_or_else(|| CostError::Overflow {
                model: model.to_owned(),
            })
    }
}

fn default_setup() -> EndpointSetup {
    EndpointSetup {
        harness: Harness::ClaudeCode,
        model: Some("gpt-4o".to_owned()),
        gateway: None,
        api_key: None,
        api_base: None,
        metadata: JsonObject::new(),
    }
}

fn model_endpoint_with(setup: EndpointSetup, prices: Arc<dyn CostPerToken>) -> ModelEndpoint {
    ModelEndpoint::new(setup, prices, &FixedRandom(0xFB)).unwrap()
}

fn model_endpoint(prices: Arc<dyn CostPerToken>) -> ModelEndpoint {
    model_endpoint_with(default_setup(), prices)
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn priced_endpoint() -> ModelEndpoint {
    model_endpoint(Arc::new(FakePrices {
        per_token: dec("0.001"),
        fail: false,
    }))
}

fn parse(text: &str) -> (u64, u64) {
    let mut parser = SSEUsageParser::default();
    parser.feed(text.as_bytes());
    parser.close();
    (parser.input_tokens, parser.output_tokens)
}

fn parse_split(text: &str, width: usize) -> (u64, u64) {
    let mut parser = SSEUsageParser::default();
    text.as_bytes()
        .chunks(width)
        .for_each(|chunk| parser.feed(chunk));
    parser.close();
    (parser.input_tokens, parser.output_tokens)
}

#[test]
fn usage_tracker_add_increments_tokens_cost_and_calls() {
    let mut tracker = UsageTracker::default();
    tracker.add(10, 5, dec("0.5"));
    tracker.add(1, 2, dec("0.25"));
    assert_eq!(tracker.input_tokens, 11);
    assert_eq!(tracker.output_tokens, 7);
    assert_eq!(tracker.calls, 2);
    assert_eq!(tracker.cost, dec("0.75"));
}

#[test]
fn usage_tracker_snapshot_of_a_fresh_tracker_has_zero_calls() {
    let tracker = UsageTracker::default();
    assert_eq!(
        tracker.snapshot(),
        Usage {
            input_tokens: 0,
            output_tokens: 0,
            calls: 0,
        }
    );
}

#[test]
fn usage_tracker_snapshot_carries_the_running_calls() {
    let mut tracker = UsageTracker::default();
    tracker.add(3, 4, dec("1"));
    assert_eq!(
        tracker.snapshot(),
        Usage {
            input_tokens: 3,
            output_tokens: 4,
            calls: 1,
        }
    );
}

#[rstest]
#[case::anthropic(r#"{"input_tokens":12,"output_tokens":34}"#, 12, 34)]
#[case::openai(r#"{"prompt_tokens":5,"completion_tokens":6}"#, 5, 6)]
#[case::mixed(r#"{"input_tokens":7,"completion_tokens":8}"#, 7, 8)]
#[case::float_truncates(r#"{"input_tokens":7.9,"output_tokens":2.1}"#, 7, 2)]
#[case::bool_is_zero(r#"{"input_tokens":true,"output_tokens":false}"#, 0, 0)]
#[case::present_null_does_not_fall_back(
    r#"{"input_tokens":null,"prompt_tokens":99,"output_tokens":null,"completion_tokens":99}"#,
    0,
    0
)]
#[case::text_is_zero(r#"{"input_tokens":"12","output_tokens":"3"}"#, 0, 0)]
fn usage_from_mapping_reads_both_field_families(
    #[case] json: &str,
    #[case] input_tokens: u64,
    #[case] output_tokens: u64,
) {
    let usage = serde_json::from_str::<TokenCounts>(json).unwrap();
    let (input, output) = usage_from_mapping(&usage);
    assert_eq!(input, input_tokens);
    assert_eq!(output, output_tokens);
}

#[rstest]
#[case::top_level(r#"{"usage":{"prompt_tokens":3,"completion_tokens":4}}"#, 3, 4)]
#[case::responses(r#"{"response":{"usage":{"input_tokens":5,"output_tokens":6}}}"#, 5, 6)]
#[case::prefers_top_level(
    r#"{"usage":{"input_tokens":1,"output_tokens":2},"response":{"usage":{"input_tokens":7,"output_tokens":8}}}"#,
    1,
    2
)]
#[case::no_usage(r#"{"id":"x"}"#, 0, 0)]
#[case::unusable_usage(r#"{"usage":[1,2]}"#, 0, 0)]
#[case::unusable_response(r#"{"response":"none"}"#, 0, 0)]
fn usage_from_body_reads_the_json_response(
    #[case] json: &str,
    #[case] input_tokens: u64,
    #[case] output_tokens: u64,
) {
    assert_eq!(
        usage_from_body(json.as_bytes()),
        (input_tokens, output_tokens)
    );
}

#[test]
fn sse_parser_reads_anthropic_message_start() {
    let tokens = parse(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":1}}}\n\n",
    );
    assert_eq!(tokens, (12, 1));
}

#[test]
fn sse_parser_reads_cumulative_message_delta() {
    let tokens = parse(
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\
         data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":20}}\n\
         data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":35}}\n",
    );
    assert_eq!(tokens, (10, 35));
}

#[test]
fn sse_parser_keeps_the_last_nonzero_count() {
    let tokens = parse(
        "data: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20}}\n\
         data: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":0,\"output_tokens\":0}}\n",
    );
    assert_eq!(tokens, (10, 20));
}

#[test]
fn sse_parser_reads_response_completed() {
    let tokens = parse(
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":5,\"output_tokens\":6}}}\n",
    );
    assert_eq!(tokens, (5, 6));
}

#[test]
fn sse_parser_reads_openai_usage_objects() {
    let tokens =
        parse("data: {\"id\":\"x\",\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":8}}\n");
    assert_eq!(tokens, (7, 8));
}

#[test]
fn sse_parser_ignores_the_done_trailer() {
    let tokens = parse(
        "data: {\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":8}}\n\
         data: [DONE]\n\n",
    );
    assert_eq!(tokens, (7, 8));
}

#[test]
fn sse_parser_accepts_data_without_a_space() {
    assert_eq!(parse("data:{\"usage\":{\"input_tokens\":3}}\n"), (3, 0));
}

#[test]
fn sse_parser_skips_malformed_and_non_object_events() {
    let tokens = parse(
        "data: {not json}\n\
         data: [1,2]\n\
         data: [null,null,null,{\"input_tokens\":5}]\n\
         data: \"text\"\n\
         data: {\"usage\":{\"input_tokens\":4}}\n",
    );
    assert_eq!(tokens, (4, 0));
}

#[test]
fn sse_parser_reads_usage_of_an_event_with_a_non_text_type() {
    let tokens = parse(
        "data: {\"type\":5,\"usage\":{\"input_tokens\":3}}\n\
         data: {\"type\":null,\"message\":7,\"usage\":{\"output_tokens\":2}}\n",
    );
    assert_eq!(tokens, (3, 2));
}

#[test]
fn sse_parser_ignores_lines_without_a_data_prefix() {
    let tokens = parse("event: message_start\n:keep-alive\n{\"input_tokens\":9}\n");
    assert_eq!(tokens, (0, 0));
}

#[test]
fn sse_parser_ignores_a_message_start_without_a_message() {
    let tokens = parse(
        "data: {\"type\":\"message_start\",\"message\":\"none\",\"usage\":{\"input_tokens\":3}}\n",
    );
    assert_eq!(tokens, (0, 0));
}

#[test]
fn sse_parser_reads_usage_across_split_feeds() {
    let text =
        "data: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":11,\"output_tokens\":22}}\n";
    assert_eq!(parse_split(text, 3), (11, 22));
    assert_eq!(parse_split(text, 1), parse(text));
}

#[test]
fn sse_parser_close_reads_a_line_without_a_newline() {
    let mut parser = SSEUsageParser::default();
    parser.feed(b"data: {\"usage\":{\"input_tokens\":9,\"output_tokens\":4}}");
    assert_eq!((parser.input_tokens, parser.output_tokens), (0, 0));
    parser.close();
    assert_eq!((parser.input_tokens, parser.output_tokens), (9, 4));
}

#[test]
fn sse_parser_close_without_a_remainder_changes_nothing() {
    let mut parser = SSEUsageParser::default();
    parser.feed(b"data: {\"usage\":{\"input_tokens\":9}}\n");
    parser.close();
    parser.close();
    assert_eq!((parser.input_tokens, parser.output_tokens), (9, 0));
}

#[rstest]
fn replays_the_sse_parser_corpus(#[files("fuzz/corpus/sse_parser/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_stem().unwrap().to_str().unwrap();
    let (_, expected) = name.rsplit_once('~').unwrap();
    let (input_text, output_text) = expected.split_once('+').unwrap();
    let input: u64 = input_text.parse().unwrap();
    let output: u64 = output_text.parse().unwrap();
    let mut whole = SSEUsageParser::default();
    whole.feed(&bytes);
    whole.close();
    let mut split = SSEUsageParser::default();
    bytes.chunks(3).for_each(|chunk| split.feed(chunk));
    split.close();
    assert_eq!((whole.input_tokens, whole.output_tokens), (input, output));
    assert_eq!((split.input_tokens, split.output_tokens), (input, output));
}

#[test]
fn compute_cost_is_zero_without_a_model() {
    let prices = FixedPrices(dec("1"));
    assert_eq!(compute_cost(&prices, None, 10, 10), Decimal::ZERO);
}

#[test]
fn compute_cost_is_zero_without_tokens() {
    let prices = FixedPrices(dec("1"));
    assert_eq!(compute_cost(&prices, Some("gpt-4o"), 0, 0), Decimal::ZERO);
}

#[test]
fn compute_cost_prices_a_call_with_only_one_token_kind() {
    let prices = FakePrices {
        per_token: dec("1"),
        fail: false,
    };
    assert_eq!(compute_cost(&prices, Some("gpt-4o"), 0, 2), dec("2"));
    assert_eq!(compute_cost(&prices, Some("gpt-4o"), 3, 0), dec("3"));
}

#[test]
fn compute_cost_adds_the_input_and_output_charges() {
    let prices = FakePrices {
        per_token: dec("0.5"),
        fail: false,
    };
    assert_eq!(compute_cost(&prices, Some("gpt-4o"), 4, 2), dec("3"));
}

#[test]
fn compute_cost_is_zero_when_the_price_lookup_fails() {
    let prices = FakePrices {
        per_token: dec("1"),
        fail: true,
    };
    assert_eq!(compute_cost(&prices, Some("unknown"), 1, 1), Decimal::ZERO);
}

#[test]
fn bundled_catalog_prices_a_known_model() {
    let prices = CatalogPrices::bundled(fixed_clock());
    assert_eq!(
        compute_cost(&prices, Some("gpt-4o"), 1_000, 500),
        dec("0.0075")
    );
}

#[test]
fn bundled_catalog_answers_the_price_lookups() {
    let prices = CatalogPrices::bundled(fixed_clock());
    let effective = prices
        .effective_model_info("gpt-4o", Some("openai"))
        .unwrap();
    assert!(effective.contains_key("input_cost_per_token"));
    assert!(
        prices
            .effective_model_info("not-a-model-in-the-catalog", None)
            .is_none()
    );
    let direct = prices.raw_cost_map_entry("gpt-4o").unwrap();
    assert!(direct.contains_key("output_cost_per_token"));
    assert!(
        prices
            .raw_cost_map_entry("not-a-model-in-the-catalog")
            .is_none()
    );
    let model = prices.effective_priced_model("gpt-4o", None).unwrap();
    assert!(model.record.contains_key("input_cost_per_token"));
    assert!(model.tiered_pricing.is_empty());
    assert!(model.off_peak_pricing.is_none());
    assert!(
        prices
            .effective_priced_model("not-a-model-in-the-catalog", None)
            .is_none()
    );
}

#[test]
fn bundled_catalog_prices_an_unknown_model_at_zero() {
    let prices = CatalogPrices::bundled(fixed_clock());
    assert_eq!(
        compute_cost(&prices, Some("not-a-model-in-the-catalog"), 10, 10),
        Decimal::ZERO
    );
}

#[rstest]
#[case::plain("0.0012", Some("0.0012"))]
#[case::spaced(" 0.5 ", Some("0.5"))]
#[case::scientific("1e-3", Some("0.001"))]
#[case::invalid("not-a-number", None)]
#[case::infinite("inf", None)]
fn header_cost_reads_the_cost_header(#[case] raw: &str, #[case] expected: Option<&str>) {
    let map = headers(&[(super::cost::COST_HEADER, raw)]);
    assert_eq!(header_cost(&map), expected.map(dec));
}

#[test]
fn header_cost_is_none_without_the_header() {
    assert_eq!(header_cost(&HeaderMap::new()), None);
}

#[rstest]
#[case::number(r#"{"response_cost":0.25}"#, Some("0.25"))]
#[case::integer(r#"{"response_cost":2}"#, Some("2"))]
#[case::null(r#"{"response_cost":null}"#, None)]
#[case::missing(r#"{"other":1}"#, None)]
#[case::text(r#"{"response_cost":"0.25"}"#, None)]
#[case::boolean(r#"{"response_cost":true}"#, None)]
fn hidden_cost_reads_the_response_cost(#[case] json: &str, #[case] expected: Option<&str>) {
    assert_eq!(hidden_cost(&object(json)), expected.map(dec));
}

#[rstest]
#[case::bearer(Some("Bearer abc"), None, Some("abc"))]
#[case::lowercase_scheme(Some("bearer abc"), None, Some("abc"))]
#[case::uppercase_scheme(Some("BEARER abc"), None, Some("abc"))]
#[case::trims_token(Some("Bearer   abc  "), None, Some("abc"))]
#[case::api_key(None, Some("sk-abc"), Some("sk-abc"))]
#[case::bearer_wins(Some("Bearer abc"), Some("sk-key"), Some("abc"))]
#[case::no_headers(None, None, None)]
#[case::no_space(Some("Bearerabc"), None, None)]
#[case::other_scheme(Some("Basic abc"), None, None)]
#[case::empty_bearer(Some("Bearer   "), None, Some(""))]
fn extract_token_reads_bearer_or_api_key(
    #[case] authorization: Option<&str>,
    #[case] api_key: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut map = HeaderMap::new();
    if let Some(value) = authorization {
        map.insert(AUTHORIZATION, HeaderValue::from_str(value).unwrap());
    }
    if let Some(value) = api_key {
        map.insert("x-api-key", HeaderValue::from_str(value).unwrap());
    }
    let token = extract_token(&map);
    assert_eq!(token.as_ref().map(ExposeSecret::expose_secret), expected);
}

#[test]
fn gateway_headers_drop_credentials_hop_by_hop_and_litellm_headers() {
    let incoming = headers(&[
        ("content-type", "application/json"),
        ("user-agent", "claude-cli"),
        ("authorization", "Bearer session-token"),
        ("x-api-key", "sk-incoming"),
        ("accept-encoding", "gzip"),
        ("connection", "keep-alive"),
        ("host", "127.0.0.1:1"),
        ("x-litellm-tags", "old"),
        ("x-litellm-spend-logs-metadata", "{}"),
    ]);
    let outgoing = gateway_headers(
        &incoming,
        &gateway(),
        Harness::ClaudeCode,
        &JsonObject::new(),
    )
    .unwrap();
    assert_eq!(header_text(&outgoing, "content-type"), "application/json");
    assert_eq!(header_text(&outgoing, "user-agent"), "claude-cli");
    assert_eq!(header_text(&outgoing, "authorization"), "Bearer sk-gateway");
    assert!(outgoing.get("authorization").unwrap().is_sensitive());
    assert_eq!(
        header_text(&outgoing, "x-litellm-tags"),
        "harness,claude_code"
    );
    assert!(outgoing.get("x-api-key").is_none());
    assert!(outgoing.get("accept-encoding").is_none());
    assert!(outgoing.get("connection").is_none());
    assert!(outgoing.get("host").is_none());
    assert!(outgoing.get("x-litellm-spend-logs-metadata").is_none());
}

#[rstest]
#[case::claude_code(Harness::ClaudeCode, "harness,claude_code")]
#[case::codex(Harness::Codex, "harness,codex")]
#[case::opencode(Harness::OpenCode, "harness,opencode")]
#[case::deepagents(Harness::DeepAgents, "harness,deepagents")]
fn gateway_headers_tag_each_harness(#[case] harness: Harness, #[case] expected: &str) {
    let outgoing =
        gateway_headers(&HeaderMap::new(), &gateway(), harness, &JsonObject::new()).unwrap();
    assert_eq!(
        outgoing.get("x-litellm-tags").unwrap().to_str().unwrap(),
        expected
    );
}

#[test]
fn gateway_headers_carry_the_session_metadata() {
    let metadata = object(r#"{"team":"core","run":7}"#);
    let outgoing =
        gateway_headers(&HeaderMap::new(), &gateway(), Harness::Codex, &metadata).unwrap();
    assert_eq!(
        outgoing
            .get("x-litellm-spend-logs-metadata")
            .unwrap()
            .to_str()
            .unwrap(),
        r#"{"run": 7, "team": "core"}"#
    );
}

#[test]
fn gateway_headers_escape_non_ascii_metadata_so_the_header_stays_valid() {
    let metadata = object(r#"{"team":"\u00fc \u2713"}"#);
    let outgoing =
        gateway_headers(&HeaderMap::new(), &gateway(), Harness::Codex, &metadata).unwrap();
    assert_eq!(
        header_text(&outgoing, "x-litellm-spend-logs-metadata"),
        r#"{"team": "\u00fc \u2713"}"#
    );
}

#[test]
fn response_headers_keep_end_to_end_headers_only() {
    let upstream = headers(&[
        ("content-type", "text/event-stream"),
        ("x-request-id", "abc"),
        ("content-length", "12"),
        ("transfer-encoding", "chunked"),
        ("content-encoding", "gzip"),
        ("connection", "close"),
    ]);
    let kept = response_headers(&upstream);
    assert_eq!(
        kept.get("content-type").unwrap().to_str().unwrap(),
        "text/event-stream"
    );
    assert_eq!(kept.get("x-request-id").unwrap().to_str().unwrap(), "abc");
    assert!(kept.get("content-length").is_none());
    assert!(kept.get("transfer-encoding").is_none());
    assert!(kept.get("content-encoding").is_none());
    assert!(kept.get("connection").is_none());
}

#[test]
fn response_headers_keep_repeated_headers() {
    let upstream = headers(&[("set-cookie", "a=1"), ("set-cookie", "b=2")]);
    let kept = response_headers(&upstream);
    assert_eq!(kept.get_all("set-cookie").iter().count(), 2);
}

#[test]
fn sanitize_replaces_every_secret() {
    let (gateway_key, api_key, session) = (
        secret("sk-gateway"),
        secret("sk-api"),
        secret("secret-token"),
    );
    assert_eq!(
        sanitize(
            "gateway sk-gateway api sk-api token secret-token",
            &[Some(&gateway_key), Some(&api_key), Some(&session)],
        ),
        "gateway *** api *** token ***"
    );
}

#[test]
fn sanitize_skips_absent_and_empty_secrets() {
    assert_eq!(
        sanitize("nothing to hide", &[None, Some(&secret(""))]),
        "nothing to hide"
    );
}

#[test]
fn sanitize_replaces_in_the_order_the_secrets_are_listed() {
    // replacing the shorter secret first leaves a suffix the longer one no longer matches
    let (short, long) = (secret("sk-abc"), secret("sk-abc-def"));
    assert_eq!(
        sanitize("sk-abc-def", &[Some(&short), Some(&long)]),
        "***-def"
    );
    assert_eq!(sanitize("sk-abc-def", &[Some(&long), Some(&short)]), "***");
}

#[test]
fn unauthorized_reply_is_a_401_authentication_error() {
    let reply = ModelEndpoint::unauthorized();
    assert_eq!(reply.status, 401);
    assert_eq!(reply.body.error.error_type, "authentication_error");
    assert_eq!(reply.body.error.message, "invalid token");
}

#[rstest]
#[case::missing_status(None, None, 500)]
#[case::too_low(None, Some(399), 500)]
#[case::lowest_accepted(None, Some(400), 400)]
#[case::highest_accepted(None, Some(599), 599)]
#[case::too_high(None, Some(600), 500)]
#[case::own_status_used(None, Some(503), 503)]
#[case::explicit_status_wins(Some(502), Some(503), 502)]
#[case::explicit_zero_falls_back(Some(0), Some(503), 503)]
#[case::explicit_status_without_own(Some(400), None, 400)]
fn error_reply_status_follows_the_failure(
    #[case] explicit: Option<u16>,
    #[case] own: Option<u16>,
    #[case] expected: u16,
) {
    let endpoint = priced_endpoint();
    let failure = Failure {
        error_type: "APIError".to_owned(),
        message: "upstream failed".to_owned(),
        status: own,
    };
    assert_eq!(endpoint.error(&failure, explicit).status, expected);
}

#[test]
fn error_reply_keeps_the_error_type_and_redacts_secrets() {
    let mut setup = default_setup();
    setup.gateway = Some(gateway());
    setup.api_key = Some(SecretString::from("sk-api".to_owned()));
    let endpoint = model_endpoint_with(
        setup,
        Arc::new(FakePrices {
            per_token: dec("1"),
            fail: false,
        }),
    );
    let token = endpoint.token.expose_secret().to_owned();
    let failure = Failure {
        error_type: "AuthenticationError".to_owned(),
        message: format!("sk-gateway sk-api {token} upstream failed"),
        status: Some(401),
    };
    let reply = endpoint.error(&failure, None);
    assert_eq!(reply.status, 401);
    assert_eq!(reply.body.error.error_type, "AuthenticationError");
    assert_eq!(reply.body.error.message, "*** *** *** upstream failed");
}

#[test]
fn endpoint_generates_a_url_safe_session_token() {
    let endpoint = priced_endpoint();
    let token = endpoint.token.expose_secret();
    assert_eq!(token, format!("{}-_s", "-_v7".repeat(10)));
    assert_eq!(HARNESS_SESSION_TOKEN_BYTES, 32);
}

#[test]
fn endpoint_fails_when_the_random_source_fails() {
    let result = ModelEndpoint::new(
        default_setup(),
        Arc::new(FixedPrices(Decimal::ZERO)),
        &FailingRandom,
    );
    assert!(matches!(result, Err(EndpointError::Token { .. })));
}

#[test]
fn system_random_fills_the_whole_buffer_differently_each_time() {
    let mut first = [0_u8; HARNESS_SESSION_TOKEN_BYTES];
    let mut second = [0_u8; HARNESS_SESSION_TOKEN_BYTES];
    SystemRandom.fill(&mut first).unwrap();
    SystemRandom.fill(&mut second).unwrap();
    assert_ne!(first, [0_u8; HARNESS_SESSION_TOKEN_BYTES]);
    assert_ne!(first, second);
}

#[test]
fn system_clock_reads_the_current_time() {
    let before = Utc::now();
    let reading = SystemClock.now();
    assert!(reading >= before);
    assert!(reading <= Utc::now());
}

#[test]
fn endpoint_authorizes_the_bearer_token_and_the_api_key() {
    let endpoint = priced_endpoint();
    let token = endpoint.token.expose_secret().to_owned();
    let mut bearer = HeaderMap::new();
    bearer.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("bearer {token}")).unwrap(),
    );
    assert!(endpoint.authorized(&bearer));
    let mut api_key = HeaderMap::new();
    api_key.insert("x-api-key", HeaderValue::from_str(&token).unwrap());
    assert!(endpoint.authorized(&api_key));
}

#[test]
fn endpoint_rejects_a_missing_or_wrong_token() {
    let endpoint = priced_endpoint();
    assert!(!endpoint.authorized(&HeaderMap::new()));
    let mut wrong = HeaderMap::new();
    wrong.insert(
        AUTHORIZATION,
        HeaderValue::from_str("Bearer wrong").unwrap(),
    );
    assert!(!endpoint.authorized(&wrong));
    let mut partial = HeaderMap::new();
    let token = endpoint.token.expose_secret().to_owned();
    partial.insert(
        AUTHORIZATION,
        HeaderValue::from_str(token.get(1..).unwrap()).unwrap(),
    );
    assert!(!endpoint.authorized(&partial));
}

#[test]
fn endpoint_secrets_list_gateway_api_and_session_credentials() {
    let mut setup = default_setup();
    setup.gateway = Some(gateway());
    setup.api_key = Some(SecretString::from("sk-api".to_owned()));
    let endpoint = model_endpoint_with(
        setup,
        Arc::new(FakePrices {
            per_token: dec("1"),
            fail: false,
        }),
    );
    let token = endpoint.token.expose_secret().to_owned();
    let [gateway_key, api_key, session] = endpoint.secrets();
    assert_eq!(gateway_key.unwrap().expose_secret(), "sk-gateway");
    assert_eq!(api_key.unwrap().expose_secret(), "sk-api");
    assert_eq!(session.unwrap().expose_secret(), token);
}

#[test]
fn endpoint_record_adds_tokens_cost_and_a_call() {
    let endpoint = priced_endpoint();
    endpoint.record(Some("gpt-4o"), 10, 5, Some(dec("0.5")));
    endpoint.record(Some("gpt-4o"), 1, 2, Some(dec("0.25")));
    assert_eq!(
        endpoint.usage(),
        Usage {
            input_tokens: 11,
            output_tokens: 7,
            calls: 2,
        }
    );
    assert_eq!(endpoint.cost(), dec("0.75"));
}

#[test]
fn endpoint_record_computes_the_cost_when_the_response_has_none() {
    let endpoint = priced_endpoint();
    endpoint.record(Some("gpt-4o"), 10, 5, None);
    assert_eq!(endpoint.cost(), dec("0.015"));
    assert_eq!(endpoint.usage().calls, 1);
}

#[test]
fn endpoint_record_without_a_model_costs_zero() {
    let endpoint = priced_endpoint();
    endpoint.record(None, 10, 5, None);
    assert_eq!(endpoint.cost(), Decimal::ZERO);
    assert_eq!(endpoint.usage().calls, 1);
}

#[rstest]
#[case::anthropic_json(
    Route::Messages,
    r#"{"type":"message_start"}"#,
    b"event: message_start\ndata: {\"type\":\"message_start\"}\n\n".as_slice()
)]
#[case::anthropic_default_type(
    Route::Messages,
    r#"{"other":1}"#,
    b"event: message\ndata: {\"other\":1}\n\n".as_slice()
)]
#[case::chat_json(
    Route::Chat,
    r#"{"choices":[]}"#,
    b"data: {\"choices\":[]}\n\n".as_slice()
)]
#[case::responses_json(
    Route::Responses,
    r#"{"type":"response.output_text.delta"}"#,
    b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\"}\n\n".as_slice()
)]
fn encode_chunk_frames_json_events(
    #[case] route: Route,
    #[case] json: &'static str,
    #[case] expected: &'static [u8],
) {
    let event = object(json);
    assert_eq!(encode_chunk(route, Chunk::Json(&event)).unwrap(), expected);
}

#[test]
fn encode_anthropic_chunk_passes_raw_chunks_through() {
    assert_eq!(
        encode_chunk(Route::Messages, Chunk::Bytes(b"raw")).unwrap(),
        b"raw"
    );
    assert_eq!(
        encode_chunk(Route::Messages, Chunk::Text("raw")).unwrap(),
        b"raw"
    );
}

#[test]
fn encode_chat_chunk_wraps_text_as_a_json_string() {
    assert_eq!(
        encode_chunk(Route::Chat, Chunk::Text("hello")).unwrap(),
        b"data: \"hello\"\n\n"
    );
}

#[test]
fn encode_responses_chunk_wraps_text_as_a_message_event() {
    assert_eq!(
        encode_chunk(Route::Responses, Chunk::Text("hello")).unwrap(),
        b"event: message\ndata: \"hello\"\n\n"
    );
}

#[test]
fn encode_chunk_rejects_bytes_on_routes_without_a_raw_form() {
    assert!(matches!(
        encode_chunk(Route::Chat, Chunk::Bytes(b"raw")),
        Err(EncodeError::BytesNotJson {
            route: "chat/completions"
        })
    ));
    assert!(matches!(
        encode_chunk(Route::Responses, Chunk::Bytes(b"raw")),
        Err(EncodeError::BytesNotJson { route: "responses" })
    ));
}

#[rstest]
#[case::newline(
    r#"{"type":"ok\ndata: [DONE]"}"#,
    b"event: message\ndata: {\"type\":\"ok\\ndata: [DONE]\"}\n\n".as_slice()
)]
#[case::empty(r#"{"type":""}"#, b"event: message\ndata: {\"type\":\"\"}\n\n".as_slice())]
#[case::carriage_return(
    r#"{"type":"ok\rdata"}"#,
    b"event: message\ndata: {\"type\":\"ok\\rdata\"}\n\n".as_slice()
)]
#[case::number(r#"{"type":7}"#, b"event: message\ndata: {\"type\":7}\n\n".as_slice())]
fn encode_chunk_never_puts_a_foreign_event_name_on_the_event_line(
    #[case] json: &'static str,
    #[case] expected: &'static [u8],
) {
    let event = object(json);
    assert_eq!(
        encode_chunk(Route::Messages, Chunk::Json(&event)).unwrap(),
        expected
    );
}

#[test]
fn stream_trailer_is_the_chat_done_marker_only() {
    assert_eq!(
        stream_trailer(Route::Chat),
        Some(b"data: [DONE]\n\n".as_slice())
    );
    assert_eq!(stream_trailer(Route::Messages), None);
    assert_eq!(stream_trailer(Route::Responses), None);
}

#[rstest]
#[case::v1_messages("/v1/messages", Some(Route::Messages))]
#[case::messages("/messages", Some(Route::Messages))]
#[case::bare_v1_messages("v1/messages", Some(Route::Messages))]
#[case::v1_chat("/v1/chat/completions", Some(Route::Chat))]
#[case::chat("/chat/completions", Some(Route::Chat))]
#[case::v1_responses("/v1/responses", Some(Route::Responses))]
#[case::responses("/responses", Some(Route::Responses))]
#[case::extra_slashes("//v1/messages//", Some(Route::Messages))]
#[case::not_v1("/v2/messages", None)]
#[case::bare_v1("/v1", None)]
#[case::doubled_prefix("/v1/v1/messages", None)]
#[case::other_route("/v1/embeddings", None)]
#[case::empty("", None)]
fn route_of_strips_the_prefixes(#[case] path: &str, #[case] expected: Option<Route>) {
    assert_eq!(route_of(path), expected);
}

#[test]
fn post_routes_are_the_three_streaming_routes() {
    assert_eq!(
        POST_ROUTES,
        [Route::Messages, Route::Chat, Route::Responses]
    );
    assert_eq!(
        POST_ROUTES.map(Route::path),
        ["messages", "chat/completions", "responses"]
    );
    assert_eq!(ROUTE_PREFIXES, ["", "/v1"]);
}
