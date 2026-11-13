#![cfg(test)]

mod support;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use http::{HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;
use rust_decimal::Decimal;

use self::support::{
    Fixture, RecordingDispatch, call, capture_logs, chat_json, chat_response, logging_state,
    lossy_text, payload, time,
};
use super::cost_per_request::set_cost_per_request;
use super::dispatch::{PassThroughLogDispatch, handle_logging};
use super::logger::PassThroughEndpointLogging;
use super::response::{UpstreamBodyText, safe_response_text};
use super::usage::{
    PassThroughCallState, UPSTREAM_REPORTED_USAGE_KEY, UPSTREAM_RESPONSE_COST_HEADER,
    UPSTREAM_TOTAL_TOKENS_HEADER, UpstreamReportedUsage, apply_upstream_reported_usage,
    has_upstream_reported_usage, parse_response_cost, parse_total_tokens,
    parse_upstream_reported_usage,
};
use crate::logging_state::correlation::CorrelationContext;
use crate::logging_success::assembled::ASYNC_STREAM_RESPONSE_KEY;
use crate::logging_success::payload_state::{STANDARD_LOGGING_OBJECT, standard_logging_object};

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).expect("valid header name"),
            HeaderValue::from_str(value).expect("valid header value"),
        );
    }
    map
}

fn capture_warning(run: &dyn Fn()) -> String {
    let (logs, _guard) = capture_logs();
    run();
    logs.text()
}

#[test]
fn usage_parse_returns_none_when_neither_header_is_sent() {
    let parsed = parse_upstream_reported_usage(&headers(&[("x-other", "1")]));
    assert!(parsed.is_none());
}

#[test]
fn usage_parse_reads_both_reported_headers() {
    let parsed = parse_upstream_reported_usage(&headers(&[
        (UPSTREAM_RESPONSE_COST_HEADER, "0.5"),
        (UPSTREAM_TOTAL_TOKENS_HEADER, "7"),
    ]));
    let usage = parsed.expect("reported usage");
    assert_eq!(
        usage.response_cost.map(f64::to_bits),
        Some(0.5_f64.to_bits())
    );
    assert_eq!(usage.total_tokens, Some(7));
}

#[test]
fn usage_parse_reads_a_cost_without_tokens() {
    let parsed =
        parse_upstream_reported_usage(&headers(&[(UPSTREAM_RESPONSE_COST_HEADER, "1.25")]));
    let usage = parsed.expect("reported usage");
    assert_eq!(
        usage.response_cost.map(f64::to_bits),
        Some(1.25_f64.to_bits())
    );
    assert!(usage.total_tokens.is_none());
}

#[test]
fn usage_parse_reads_tokens_without_a_cost() {
    let parsed = parse_upstream_reported_usage(&headers(&[(UPSTREAM_TOTAL_TOKENS_HEADER, "9")]));
    let usage = parsed.expect("reported usage");
    assert!(usage.response_cost.is_none());
    assert_eq!(usage.total_tokens, Some(9));
}

#[test]
fn usage_parse_matches_header_names_case_insensitively() {
    let parsed = parse_upstream_reported_usage(&headers(&[
        ("X-Litellm-Response-Cost", "2"),
        ("X-LITELLM-TOTAL-TOKENS", "3"),
    ]));
    let usage = parsed.expect("reported usage");
    assert_eq!(
        usage.response_cost.map(f64::to_bits),
        Some(2.0_f64.to_bits())
    );
    assert_eq!(usage.total_tokens, Some(3));
}

#[test]
fn usage_parse_joins_repeated_headers_with_comma_space() {
    let logs = capture_warning(&|| {
        let parsed = parse_upstream_reported_usage(&headers(&[
            (UPSTREAM_RESPONSE_COST_HEADER, "0.5"),
            (UPSTREAM_RESPONSE_COST_HEADER, "0.5"),
        ]));
        let usage = parsed.expect("the marker is stored for unusable values");
        assert!(usage.response_cost.is_none());
    });
    assert!(
        logs.contains(concat!(
            "un",
            "parseable x-litellm-response-cost='0.5, 0.5'"
        )),
        "{logs}"
    );
}

#[test]
fn usage_parse_decodes_utf8_header_bytes() {
    let mut map = HeaderMap::new();
    map.append(
        HeaderName::from_static(UPSTREAM_RESPONSE_COST_HEADER),
        HeaderValue::from_bytes("\u{e9}".as_bytes()).expect("opaque header bytes"),
    );
    let logs = capture_warning(&|| {
        assert!(parse_upstream_reported_usage(&map).is_some());
    });
    assert!(
        logs.contains(concat!("un", "parseable x-litellm-response-cost='\u{e9}'")),
        "{logs}"
    );
}

#[test]
fn usage_parse_decodes_latin1_when_the_header_map_is_not_utf8() {
    let mut map = HeaderMap::new();
    map.append(
        HeaderName::from_static(UPSTREAM_RESPONSE_COST_HEADER),
        HeaderValue::from_bytes(&[0xC3, 0xA9, 0xE9]).expect("opaque header bytes"),
    );
    let logs = capture_warning(&|| {
        assert!(parse_upstream_reported_usage(&map).is_some());
    });
    assert!(
        logs.contains(concat!(
            "un",
            "parseable x-litellm-response-cost='\u{c3}\u{a9}\u{e9}'"
        )),
        "{logs}"
    );
}

#[test]
fn usage_parse_decodes_the_whole_map_with_http_latin1() {
    let mut map = HeaderMap::new();
    map.append(
        HeaderName::from_static(UPSTREAM_RESPONSE_COST_HEADER),
        HeaderValue::from_bytes("\u{e9}".as_bytes()).expect("opaque header bytes"),
    );
    map.append(
        HeaderName::from_static(UPSTREAM_TOTAL_TOKENS_HEADER),
        HeaderValue::from_bytes(&[0xE9]).expect("opaque header bytes"),
    );
    let logs = capture_warning(&|| {
        assert!(parse_upstream_reported_usage(&map).is_some());
    });
    // httpx picks one encoding for the whole map, so the invalid byte elsewhere forces latin-1
    assert!(
        logs.contains(concat!(
            "un",
            "parseable x-litellm-response-cost='\u{c3}\u{a9}'"
        )),
        "{logs}"
    );
}

#[test]
fn cost_parse_quotes_control_characters_in_the_warning() {
    let (logs, _guard) = capture_logs();
    assert!(parse_response_cost(Some("\u{1}0")).is_none());
    let text = logs.text();
    assert!(
        text.contains(concat!("un", "parseable x-litellm-response-cost='\\x010'")),
        "{text}"
    );
}

#[rstest]
#[case::zero("0", 0.0_f64)]
#[case::negative_zero("-0", -0.0_f64)]
#[case::decimal("1.25", 1.25_f64)]
#[case::exponent("1e3", 1000.0_f64)]
#[case::signed("+2.5", 2.5_f64)]
#[case::whitespace(" 1.5 ", 1.5_f64)]
fn cost_parse_keeps_a_usable_value(#[case] raw: &str, #[case] expected: f64) {
    let parsed = parse_response_cost(Some(raw));
    assert_eq!(parsed.map(f64::to_bits), Some(expected.to_bits()));
}

#[rstest]
#[case::absent(None, "did not send x-litellm-response-cost")]
#[case::text(Some("abc"), concat!("un", "parseable x-litellm-response-cost='abc'"))]
#[case::underscores(Some("1_0"), concat!("un", "parseable x-litellm-response-cost='1_0'"))]
#[case::nan(Some("nan"), "out-of-range x-litellm-response-cost='nan'")]
#[case::infinite(Some("inf"), "out-of-range x-litellm-response-cost='inf'")]
#[case::negative(Some("-0.5"), "out-of-range x-litellm-response-cost='-0.5'")]
fn cost_parse_rejects_an_unusable_value(#[case] raw: Option<&str>, #[case] expected: &str) {
    let (logs, _guard) = capture_logs();
    assert!(parse_response_cost(raw).is_none());
    let text = logs.text();
    assert!(text.contains(expected), "{text}");
}

#[rstest]
#[case::zero("0", 0)]
#[case::negative_zero("-0", 0)]
#[case::signed("+7", 7)]
#[case::whitespace(" 42 ", 42)]
fn token_parse_keeps_a_usable_value(#[case] raw: &str, #[case] expected: i64) {
    assert_eq!(parse_total_tokens(Some(raw)), Some(expected));
}

#[rstest]
#[case::absent(None, "did not send x-litellm-total-tokens")]
#[case::decimal(Some("1.5"), concat!("un", "parseable x-litellm-total-tokens='1.5'"))]
#[case::text(Some("abc"), concat!("un", "parseable x-litellm-total-tokens='abc'"))]
#[case::underscores(Some("1_0"), concat!("un", "parseable x-litellm-total-tokens='1_0'"))]
#[case::overflow(
    Some("9223372036854775808"),
    concat!("un", "parseable x-litellm-total-tokens='9223372036854775808'")
)]
#[case::negative(Some("-3"), "negative x-litellm-total-tokens='-3'")]
fn token_parse_rejects_an_unusable_value(#[case] raw: Option<&str>, #[case] expected: &str) {
    let (logs, _guard) = capture_logs();
    assert!(parse_total_tokens(raw).is_none());
    let text = logs.text();
    assert!(text.contains(expected), "{text}");
}

#[test]
fn usage_apply_writes_only_the_reported_cost() {
    let mut logged = PassThroughCallState {
        upstream_reported_usage: None,
        response_cost: None,
        combined_usage_total_tokens: Some(7),
    };
    let applied = apply_upstream_reported_usage(
        &mut logged,
        &headers(&[(UPSTREAM_RESPONSE_COST_HEADER, "0.25")]),
    )
    .expect("reported usage");
    assert_eq!(
        applied.response_cost.map(f64::to_bits),
        Some(0.25_f64.to_bits())
    );
    assert!(applied.total_tokens.is_none());
    assert_eq!(
        logged.response_cost.map(f64::to_bits),
        Some(0.25_f64.to_bits())
    );
    assert_eq!(logged.combined_usage_total_tokens, Some(7));
    assert!(has_upstream_reported_usage(&logged));
}

#[test]
fn usage_apply_writes_only_the_reported_tokens() {
    let mut logged = PassThroughCallState {
        upstream_reported_usage: None,
        response_cost: Some(1.5_f64),
        combined_usage_total_tokens: None,
    };
    let applied = apply_upstream_reported_usage(
        &mut logged,
        &headers(&[(UPSTREAM_TOTAL_TOKENS_HEADER, "9")]),
    )
    .expect("reported usage");
    assert!(applied.response_cost.is_none());
    assert_eq!(applied.total_tokens, Some(9));
    assert_eq!(
        logged.response_cost.map(f64::to_bits),
        Some(1.5_f64.to_bits())
    );
    assert_eq!(logged.combined_usage_total_tokens, Some(9));
}

#[test]
fn usage_apply_stores_the_marker_for_unusable_values() {
    let mut logged = PassThroughCallState {
        upstream_reported_usage: None,
        response_cost: Some(1.5_f64),
        combined_usage_total_tokens: Some(3),
    };
    let applied = apply_upstream_reported_usage(
        &mut logged,
        &headers(&[(UPSTREAM_RESPONSE_COST_HEADER, "abc")]),
    )
    .expect("reported usage");
    assert!(applied.response_cost.is_none());
    assert!(applied.total_tokens.is_none());
    assert!(has_upstream_reported_usage(&logged));
    assert_eq!(
        logged.response_cost.map(f64::to_bits),
        Some(1.5_f64.to_bits())
    );
    assert_eq!(logged.combined_usage_total_tokens, Some(3));
}

#[test]
fn usage_apply_changes_nothing_without_either_header() {
    let mut logged = PassThroughCallState {
        upstream_reported_usage: None,
        response_cost: Some(2.0_f64),
        combined_usage_total_tokens: Some(4),
    };
    let applied = apply_upstream_reported_usage(&mut logged, &headers(&[("x-other", "1")]));
    assert!(applied.is_none());
    assert!(!has_upstream_reported_usage(&logged));
    assert_eq!(
        logged.response_cost.map(f64::to_bits),
        Some(2.0_f64.to_bits())
    );
    assert_eq!(logged.combined_usage_total_tokens, Some(4));
}

#[test]
fn usage_apply_marker_key_matches_litellm() {
    assert_eq!(
        UPSTREAM_REPORTED_USAGE_KEY,
        "_litellm_upstream_reported_usage"
    );
}

#[test]
fn cost_per_request_writes_both_slots() {
    let mut logged = PassThroughCallState {
        upstream_reported_usage: None,
        response_cost: None,
        combined_usage_total_tokens: None,
    };
    let mut kwargs_cost = None;
    set_cost_per_request(&mut logged, &mut kwargs_cost, Some(Decimal::new(5, 1)));
    assert_eq!(kwargs_cost.map(f64::to_bits), Some(0.5_f64.to_bits()));
    assert_eq!(
        logged.response_cost.map(f64::to_bits),
        Some(0.5_f64.to_bits())
    );
}

#[test]
fn cost_per_request_zero_counts_as_set() {
    let mut logged = PassThroughCallState {
        upstream_reported_usage: None,
        response_cost: None,
        combined_usage_total_tokens: None,
    };
    let mut kwargs_cost = None;
    set_cost_per_request(&mut logged, &mut kwargs_cost, Some(Decimal::ZERO));
    assert_eq!(kwargs_cost.map(f64::to_bits), Some(0.0_f64.to_bits()));
    assert_eq!(
        logged.response_cost.map(f64::to_bits),
        Some(0.0_f64.to_bits())
    );
}

#[test]
fn cost_per_request_overwrites_an_existing_kwargs_cost() {
    let mut logged = PassThroughCallState {
        upstream_reported_usage: None,
        response_cost: None,
        combined_usage_total_tokens: None,
    };
    let mut kwargs_cost = Some(9.0_f64);
    set_cost_per_request(&mut logged, &mut kwargs_cost, Some(Decimal::new(125, 2)));
    assert_eq!(kwargs_cost.map(f64::to_bits), Some(1.25_f64.to_bits()));
    assert_eq!(
        logged.response_cost.map(f64::to_bits),
        Some(1.25_f64.to_bits())
    );
}

#[test]
fn cost_per_request_writes_nothing_without_a_payload_cost() {
    let mut logged = PassThroughCallState {
        upstream_reported_usage: None,
        response_cost: None,
        combined_usage_total_tokens: None,
    };
    let mut kwargs_cost = Some(9.0_f64);
    set_cost_per_request(&mut logged, &mut kwargs_cost, None);
    assert_eq!(kwargs_cost.map(f64::to_bits), Some(9.0_f64.to_bits()));
    assert!(logged.response_cost.is_none());
}

#[test]
fn cost_per_request_yields_to_upstream_reported_usage() {
    let mut logged = PassThroughCallState {
        upstream_reported_usage: Some(UpstreamReportedUsage {
            response_cost: None,
            total_tokens: None,
        }),
        response_cost: None,
        combined_usage_total_tokens: None,
    };
    let mut kwargs_cost = Some(9.0_f64);
    set_cost_per_request(&mut logged, &mut kwargs_cost, Some(Decimal::new(5, 1)));
    assert_eq!(kwargs_cost.map(f64::to_bits), Some(9.0_f64.to_bits()));
    assert!(logged.response_cost.is_none());
}

#[tokio::test]
async fn dispatch_default_ignores_the_callers_cache_hit() {
    let fixture = Fixture::new(Some(payload()));
    let handler = fixture.handler();
    let mut correlation = CorrelationContext::default();
    let mut logged_state = logging_state("pass_through_endpoint", &mut correlation);
    let mut dispatch_call = call(&handler, &mut logged_state, &mut correlation);
    dispatch_call.cache_hit = true;
    dispatch_call.response = Some(chat_response());
    handle_logging(dispatch_call).await;
    assert_eq!(
        logged_state.model_call_details.get("cache_hit"),
        Some(&Json::Bool(false))
    );
    assert_eq!(
        fixture.calls.names(),
        vec!["async_log_success_event".to_owned()]
    );
}

#[tokio::test]
async fn dispatch_default_prefers_async_handlers() {
    let fixture = Fixture::new(Some(payload()));
    let handler = fixture.handler();
    let mut correlation = CorrelationContext::default();
    let mut logged_state = logging_state("completion", &mut correlation);
    let mut dispatch_call = call(&handler, &mut logged_state, &mut correlation);
    dispatch_call.response = Some(chat_response());
    handle_logging(dispatch_call).await;
    assert_eq!(
        fixture.calls.names(),
        vec!["async_log_success_event".to_owned()]
    );
}

#[tokio::test]
async fn dispatch_default_passes_the_response_and_the_same_times() {
    let fixture = Fixture::new(Some(payload()));
    let handler = fixture.handler();
    let mut correlation = CorrelationContext::default();
    let mut logged_state = logging_state("pass_through_endpoint", &mut correlation);
    let mut dispatch_call = call(&handler, &mut logged_state, &mut correlation);
    dispatch_call.response = Some(chat_response());
    handle_logging(dispatch_call).await;
    assert_eq!(
        logged_state
            .model_call_details
            .get(ASYNC_STREAM_RESPONSE_KEY),
        Some(&chat_json())
    );
    let end_time = Json::Text("2024-01-01T00:00:01+00:00".to_owned());
    assert_eq!(
        logged_state.model_call_details.get("end_time"),
        Some(&end_time)
    );
    assert_eq!(
        logged_state.model_call_details.get("completion_start_time"),
        Some(&end_time)
    );
}

#[tokio::test]
async fn dispatch_default_is_used_when_nothing_is_injected() {
    let fixture = Fixture::new(Some(payload()));
    let handler = fixture.handler();
    let logger = PassThroughEndpointLogging {
        injected_log_dispatch: None,
    };
    let mut correlation = CorrelationContext::default();
    let mut logged_state = logging_state("pass_through_endpoint", &mut correlation);
    let mut dispatch_call = call(&handler, &mut logged_state, &mut correlation);
    dispatch_call.response = Some(chat_response());
    logger.log_dispatch().dispatch(dispatch_call).await;
    assert_eq!(
        logged_state.model_call_details.get("cache_hit"),
        Some(&Json::Bool(false))
    );
    assert_eq!(
        fixture.calls.names(),
        vec!["async_log_success_event".to_owned()]
    );
}

#[tokio::test]
async fn dispatch_injected_dispatcher_replaces_the_default() {
    let fixture = Fixture::new(Some(payload()));
    let handler = fixture.handler();
    let recorder = Arc::new(RecordingDispatch::default());
    let injected: Arc<dyn PassThroughLogDispatch> = Arc::<RecordingDispatch>::clone(&recorder);
    let logger = PassThroughEndpointLogging {
        injected_log_dispatch: Some(injected),
    };
    let mut correlation = CorrelationContext::default();
    let mut logged_state = logging_state("pass_through_endpoint", &mut correlation);
    let mut dispatch_call = call(&handler, &mut logged_state, &mut correlation);
    dispatch_call.response = Some(chat_response());
    dispatch_call.result = "websocket_connection_successful";
    dispatch_call.cache_hit = true;
    dispatch_call
        .kwargs
        .entries
        .insert("marker".to_owned(), Json::Bool(true));
    logger.log_dispatch().dispatch(dispatch_call).await;
    let dispatched = recorder.recorded();
    assert_eq!(dispatched.len(), 1);
    assert_eq!(dispatched[0].start_time, time("2024-01-01T00:00:00+00:00"));
    assert_eq!(dispatched[0].end_time, time("2024-01-01T00:00:01+00:00"));
    assert_eq!(dispatched[0].result, "websocket_connection_successful");
    assert!(dispatched[0].cache_hit);
    assert_eq!(
        dispatched[0].kwargs.entries.get("marker"),
        Some(&Json::Bool(true))
    );
    assert_eq!(dispatched[0].response, Some(chat_response()));
    assert!(fixture.calls.names().is_empty());
}

#[tokio::test]
async fn dispatch_default_forwards_the_standard_logging_object_kwarg() {
    let fixture = Fixture::new(None);
    let handler = fixture.handler();
    let mut correlation = CorrelationContext::default();
    let mut logged_state = logging_state("pass_through_endpoint", &mut correlation);
    let mut dispatch_call = call(&handler, &mut logged_state, &mut correlation);
    dispatch_call.response = Some(chat_response());
    let mut forwarded = payload();
    forwarded.id = "forwarded-1".to_owned();
    let encoded = serde_json::to_string(&forwarded).expect("payload JSON");
    let value: Json = serde_json::from_str(&encoded).expect("payload object");
    dispatch_call
        .kwargs
        .entries
        .insert(STANDARD_LOGGING_OBJECT.to_owned(), value);
    handle_logging(dispatch_call).await;
    let stored = standard_logging_object(&logged_state)
        .expect("payload readable")
        .expect("payload stored");
    assert_eq!(stored.id, "forwarded-1");
}

#[test]
fn response_text_returns_the_text_of_a_read_body() {
    assert_eq!(
        safe_response_text(&UpstreamBodyText::Read("hello".to_owned())),
        "hello"
    );
}

#[test]
fn response_text_is_empty_for_an_unread_body() {
    assert_eq!(safe_response_text(&UpstreamBodyText::NotRead), "");
}

#[rstest]
#[case::messages("https://api.anthropic.com/v1/messages", true)]
#[case::batches("/v1/messages/batches", true)]
#[case::other("https://api.openai.com/v1/chat/completions", false)]
fn route_anthropic_matches_litellm_substrings(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(
        PassThroughEndpointLogging::is_anthropic_route(route),
        expected
    );
}

#[rstest]
#[case::chat("/v2/chat", true)]
#[case::embed("/v1/embed", true)]
#[case::embed_with_suffix("/v1/embed/v2", true)]
#[case::embeddings("/v1/embeddings", false)]
#[case::embeddings_inside_another_query("https://h/v1/embed?next=/v1/embeddings", false)]
fn route_cohere_skips_embeddings_routes(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(PassThroughEndpointLogging::is_cohere_route(route), expected);
}

#[rstest]
#[case::api_host("https://api.assemblyai.com/v1/other", true)]
#[case::transcript_path("https://proxy/v1/transcript/123", true)]
#[case::lookalike_host("https://api.assemblyai.com.evil/v1/other", false)]
#[case::unrelated_path("https://proxy/v1/other", false)]
fn route_assemblyai_matches_host_or_path(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(
        PassThroughEndpointLogging::is_assemblyai_route(route),
        expected
    );
}

#[test]
fn route_assemblyai_treats_a_network_path_as_a_plain_path() {
    assert!(!PassThroughEndpointLogging::is_assemblyai_route(
        "//api.assemblyai.com/v1/other"
    ));
}

#[rstest]
#[case::provider(Some("comprehendmedical"), true)]
#[case::other_provider(Some("azure_speech"), false)]
#[case::no_provider(None, false)]
fn route_comprehend_medical_matches_the_provider(
    #[case] provider: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(
        PassThroughEndpointLogging::is_comprehend_medical_route(provider),
        expected
    );
}

#[rstest]
#[case::provider(Some("azure_speech"), true)]
#[case::other_provider(Some("comprehendmedical"), false)]
#[case::no_provider(None, false)]
fn route_azure_speech_matches_the_provider(#[case] provider: Option<&str>, #[case] expected: bool) {
    assert_eq!(
        PassThroughEndpointLogging::is_azure_speech_route(provider),
        expected
    );
}

#[rstest]
#[case::provider(Some("transcribe"), true)]
#[case::other_provider(Some("typesafe"), false)]
#[case::no_provider(None, false)]
fn route_transcribe_matches_the_provider(#[case] provider: Option<&str>, #[case] expected: bool) {
    assert_eq!(
        PassThroughEndpointLogging::is_transcribe_route(provider),
        expected
    );
}

#[rstest]
#[case::provider(Some("typesafe"), true)]
#[case::other_provider(Some("transcribe"), false)]
#[case::no_provider(None, false)]
fn route_typesafe_matches_the_provider(#[case] provider: Option<&str>, #[case] expected: bool) {
    assert_eq!(
        PassThroughEndpointLogging::is_typesafe_route(provider),
        expected
    );
}

#[rstest]
#[case::plain_path("/alpha/decisions", Some("openrouter"), true)]
#[case::absolute_url("https://h/alpha/decisions", Some("openrouter"), true)]
#[case::query_is_ignored("https://h/alpha/decisions?next=1", Some("openrouter"), true)]
#[case::other_provider("/alpha/decisions", None, false)]
#[case::different_path("https://h/alpha/decision", Some("openrouter"), false)]
fn route_openrouter_decisions_needs_provider_and_path(
    #[case] route: &str,
    #[case] provider: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(
        PassThroughEndpointLogging::is_openrouter_decisions_route(route, provider),
        expected
    );
}

#[rstest]
#[case::absolute_url("https://h/langfuse/v1/traces", true)]
#[case::plain_path("/langfuse/x", true)]
#[case::bare_segment("https://h/langfuse", false)]
#[case::query_is_not_path("https://h/langfuse?next=/langfuse/", false)]
fn route_langfuse_checks_the_path_only(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(
        PassThroughEndpointLogging::is_langfuse_route(route),
        expected
    );
}

#[rstest]
#[case::plain_path("/vertex_ai/live", true)]
#[case::absolute_url("wss://h/vertex_ai/live?x=1", true)]
#[case::empty_route("", false)]
#[case::other("/vertex_ai/other", false)]
fn route_vertex_ai_live_checks_the_whole_route(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(
        PassThroughEndpointLogging::is_vertex_ai_live_route(route),
        expected
    );
}

#[rstest]
#[case::api_host("https://api.cursor.com/v0/agents", true)]
#[case::lookalike_host("https://api.cursor.com.evil/x", true)]
#[case::other_host("https://example.com/v0/agents", false)]
fn route_cursor_matches_hosts_holding_the_cursor_api(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(
        PassThroughEndpointLogging::is_cursor_route(route, None),
        expected
    );
}

#[test]
fn route_cursor_provider_shortcut_ignores_the_path() {
    let matches =
        PassThroughEndpointLogging::is_cursor_route("https://example.com/other", Some("cursor"));
    assert!(matches);
}

#[test]
fn route_cursor_tracked_path_needs_the_cursor_provider() {
    let without = PassThroughEndpointLogging::is_cursor_route("/v0/agents", None);
    assert!(!without);
    let with = PassThroughEndpointLogging::is_cursor_route("/v0/agents", Some("cursor"));
    assert!(with);
    let other = PassThroughEndpointLogging::is_cursor_route("/v0/agents", Some("other"));
    assert!(!other);
}

#[test]
fn route_vertex_tables_match_litellm() {
    assert_eq!(
        PassThroughEndpointLogging::TRACKED_VERTEX_METHOD_ROUTES,
        [
            "generateContent",
            "streamGenerateContent",
            "predict",
            "rawPredict",
            "streamRawPredict",
            "search",
            "predictLongRunning",
            "embedContent",
            "batchEmbedContents",
        ]
    );
    assert_eq!(
        PassThroughEndpointLogging::TRACKED_VERTEX_RESOURCE_ROUTES,
        ["batchPredictionJobs"]
    );
    assert_eq!(
        PassThroughEndpointLogging::TRACKED_GEMINI_ROUTES,
        [
            "generateContent",
            "streamGenerateContent",
            "predictLongRunning"
        ]
    );
    assert_eq!(
        PassThroughEndpointLogging::TRACKED_VERTEX_AI_LIVE_ROUTES,
        ["/vertex_ai/live"]
    );
}

#[test]
fn route_provider_tables_match_litellm() {
    assert_eq!(
        PassThroughEndpointLogging::TRACKED_ANTHROPIC_ROUTES,
        ["/messages", "/v1/messages/batches"]
    );
    assert_eq!(
        PassThroughEndpointLogging::TRACKED_COHERE_ROUTES,
        ["/v2/chat", "/v1/embed"]
    );
    assert_eq!(
        PassThroughEndpointLogging::TRACKED_LANGFUSE_ROUTES,
        ["/langfuse/"]
    );
    assert_eq!(
        PassThroughEndpointLogging::TRACKED_CURSOR_ROUTES,
        ["/v0/agents", "/v0/me", "/v0/models", "/v0/repositories"]
    );
}

#[rstest]
fn replays_the_passthrough_upstream_usage_corpus(
    #[files("fuzz/corpus/passthrough_upstream_usage/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    let text = lossy_text(&bytes);
    let _cost = parse_response_cost(Some(&text));
    let _tokens = parse_total_tokens(Some(&text));
}
