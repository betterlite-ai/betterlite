#![cfg(test)]

mod support;

use std::fs::read;
use std::io;
use std::path::PathBuf;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_failure::partial_usage::record_partial_usage_for_failure as record_logged_usage;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use betterlite_streaming::stream_tool_assembly::records::{FunctionOutput, ToolOutput};
use betterlite_streaming::streamed_response_assembly::response::{
    Response, TextResponse, TextUsage,
};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use self::support::{
    FailingCount, FakeCost, FixedCount, RecordingCount, completed_stream_bytes, cut_stream_bytes,
    detail_counts_response, number, recorded_cost, recorded_usage, request_body,
};
use super::lines::split_lines;
use super::testing::{
    ChatSpec, chat_response, clear_placeholder_cost as clear_cost_for_testing,
    recover_with_fixed_count, stream_was_interrupted as stream_was_interrupted_bytes,
    usage_summary,
};
use super::usage::usage_of;
use super::{
    InterruptedStreamServices, InterruptedStreamUsageError, assemble_streaming_response,
    build_streaming_response_for_logging, clear_placeholder_cost, record_partial_usage_for_failure,
    recover_interrupted_stream_output_tokens, stream_was_interrupted, usage_only_response_or_none,
};
use crate::pass_through_logging::stream_fields::Chunk;

const START: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"role\":\"assistant\",\"model\":\"claude-test\",\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\n";
const TEXT_START: &str = "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n";
const TEXT_DELTA: &str = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello world\"}}\n\n";
const CONTENT_STOP: &str =
    "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n";
const DELTA: &str = "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n";
const MESSAGE_STOP: &str = "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
const PING: &str = "event: ping\ndata: {\"type\":\"ping\"}\n\n";
const ERROR: &str = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";

// The stream a client disconnect leaves behind: the output so far and no terminal message_delta.
fn cut_stream() -> Vec<Chunk> {
    [START, TEXT_START, TEXT_DELTA]
        .iter()
        .copied()
        .map(chunk)
        .collect()
}

fn completed_stream() -> Vec<Chunk> {
    [
        START,
        TEXT_START,
        TEXT_DELTA,
        CONTENT_STOP,
        DELTA,
        MESSAGE_STOP,
    ]
    .iter()
    .copied()
    .map(chunk)
    .collect()
}

fn chunk(text: &str) -> Chunk {
    Chunk::Text(text.to_owned())
}

fn interrupted_spec() -> ChatSpec {
    ChatSpec {
        content: Some("hello world".to_owned()),
        prompt_tokens: 10,
        completion_tokens: 1,
        total_tokens: 11,
        has_details: true,
        text_tokens: Some(1),
        cost: None,
        hidden_response_cost: None,
    }
}

#[test]
fn interrupted_tail_message_delta_is_complete() {
    let stream = completed_stream();
    assert!(!stream_was_interrupted(&stream).expect("valid chunks"));
}

#[test]
fn interrupted_tail_cut_mid_content_block_delta_is_interrupted() {
    let stream = cut_stream();
    assert!(stream_was_interrupted(&stream).expect("valid chunks"));
}

#[test]
fn interrupted_tail_content_block_stop_is_interrupted() {
    let stream = [START, TEXT_DELTA, CONTENT_STOP]
        .iter()
        .copied()
        .map(chunk)
        .collect::<Vec<_>>();
    assert!(stream_was_interrupted(&stream).expect("valid chunks"));
}

#[test]
fn interrupted_tail_message_stop_only_is_interrupted() {
    let stream = vec![chunk(MESSAGE_STOP)];
    assert!(stream_was_interrupted(&stream).expect("valid chunks"));
}

#[test]
fn interrupted_tail_empty_is_interrupted() {
    assert!(stream_was_interrupted(&[]).expect("no chunks"));
}

#[test]
fn interrupted_tail_done_only_is_interrupted() {
    let stream = vec![chunk("data: [DONE]\n\n")];
    assert!(stream_was_interrupted(&stream).expect("valid chunks"));
}

#[test]
fn interrupted_tail_message_stop_after_delta_is_complete() {
    let stream = vec![chunk(DELTA), chunk(MESSAGE_STOP)];
    assert!(!stream_was_interrupted(&stream).expect("valid chunks"));
}

#[test]
fn interrupted_tail_ping_after_delta_is_interrupted() {
    let stream = vec![chunk(TEXT_DELTA), chunk(PING)];
    assert!(stream_was_interrupted(&stream).expect("valid chunks"));
}

#[test]
fn interrupted_tail_skips_bad_lines_and_non_objects() {
    let tail = concat!(
        "no data prefix here\n",
        "data: {not json}\n",
        "data: [DONE]\n",
        "data: [1, 2]\n",
        "data: 7\n",
        "data: \"text\"\n",
    );
    let stream = vec![chunk(DELTA), chunk(tail)];
    assert!(!stream_was_interrupted(&stream).expect("valid chunks"));
}

#[test]
fn interrupted_tail_skips_events_without_a_string_type() {
    let tail = concat!(
        "data: {\"type\": 7}\n",
        "data: {\"message\": \"x\"}\n",
        "data: {}\n",
    );
    let stream = vec![chunk(DELTA), chunk(tail)];
    assert!(!stream_was_interrupted(&stream).expect("valid chunks"));
}

#[rstest]
#[case(TEXT_DELTA)]
#[case(CONTENT_STOP)]
#[case(START)]
fn interrupted_tail_block_event_after_message_delta_is_interrupted(#[case] event: &str) {
    let stream = vec![chunk(DELTA), chunk(event)];
    assert!(stream_was_interrupted(&stream).expect("valid chunks"));
}

#[test]
fn interrupted_tail_line_without_a_trailing_newline_decides() {
    let stream = vec![chunk("data: {\"type\":\"message_delta\"}")];
    assert!(!stream_was_interrupted(&stream).expect("valid chunks"));
}

#[test]
fn interrupted_split_lines_follows_python_on_crlf_and_repeats() {
    assert_eq!(split_lines("a\r\nb\n\nc"), vec!["a", "b", "", "c"]);
    assert_eq!(split_lines("a\rb\r\nc"), vec!["a", "b", "c"]);
}

#[rstest]
#[case("\n")]
#[case("\r")]
#[case("\r\n")]
#[case("\u{0b}")]
#[case("\u{0c}")]
#[case("\u{1c}")]
#[case("\u{1d}")]
#[case("\u{1e}")]
#[case("\u{85}")]
#[case("\u{2028}")]
#[case("\u{2029}")]
fn interrupted_tail_splits_on_every_python_boundary(#[case] separator: &str) {
    let tail = format!("{DELTA}{separator}trailing garbage{separator}");
    let stream = vec![chunk(&tail)];
    assert!(!stream_was_interrupted(&stream).expect("valid chunks"));
}

#[rstest]
#[case(" ")]
#[case("\t")]
#[case("\u{a0}")]
#[case("\u{3000}")]
fn interrupted_tail_strips_python_whitespace(#[case] filler: &str) {
    let text = format!("data:{filler}{{\"type\":\"message_delta\"}}{filler}\n");
    let stream = vec![chunk(&text)];
    assert!(!stream_was_interrupted(&stream).expect("valid chunks"));
}

#[test]
fn interrupted_tail_decodes_byte_chunks() {
    let stream = vec![Chunk::Bytes(DELTA.as_bytes().to_vec())];
    assert!(!stream_was_interrupted(&stream).expect("valid bytes"));
}

#[test]
fn interrupted_tail_invalid_utf8_is_a_typed_error() {
    let stream = vec![Chunk::Bytes(vec![0xff, 0xfe])];
    let error = stream_was_interrupted(&stream).expect_err("invalid UTF-8");
    assert!(matches!(error, InterruptedStreamUsageError::Utf8(_)));
}

#[test]
fn interrupted_testing_wrapper_decides_cut_and_completed_streams() {
    assert!(stream_was_interrupted_bytes(&[cut_stream_bytes()]).expect("valid chunks"));
    assert!(!stream_was_interrupted_bytes(&[completed_stream_bytes()]).expect("valid chunks"));
}

#[test]
fn interrupted_recovery_raises_completion_total_and_text_tokens() {
    let outcome =
        recover_with_fixed_count(&interrupted_spec(), &[cut_stream_bytes()], "claude-test", 5)
            .expect("a valid recovery");
    assert!(outcome.changed);
    assert_eq!(outcome.usage.completion_tokens, 5);
    assert_eq!(outcome.usage.total_tokens, 15);
    assert_eq!(outcome.usage.text_tokens, Some(5));
    assert_eq!(outcome.usage.prompt_tokens, 10);
}

#[rstest]
#[case(1)]
#[case(0)]
fn interrupted_recovery_keeps_an_equal_or_lower_recount(#[case] count: i64) {
    let outcome = recover_with_fixed_count(
        &interrupted_spec(),
        &[cut_stream_bytes()],
        "claude-test",
        count,
    )
    .expect("a valid recovery");
    assert!(!outcome.changed);
    assert_eq!(outcome.usage.completion_tokens, 1);
    assert_eq!(outcome.usage.total_tokens, 11);
    assert_eq!(outcome.usage.text_tokens, Some(1));
}

#[rstest]
#[case(None)]
#[case(Some(String::new()))]
fn interrupted_recovery_skips_empty_output_text(#[case] content: Option<String>) {
    let spec = ChatSpec {
        content,
        ..interrupted_spec()
    };
    let outcome = recover_with_fixed_count(&spec, &[cut_stream_bytes()], "claude-test", 5)
        .expect("a valid recovery");
    assert!(!outcome.changed);
    assert_eq!(outcome.usage.completion_tokens, 1);
}

#[test]
fn interrupted_recovery_skips_a_completed_stream() {
    let outcome = recover_with_fixed_count(
        &interrupted_spec(),
        &[completed_stream_bytes()],
        "claude-test",
        5,
    )
    .expect("a valid recovery");
    assert!(!outcome.changed);
    assert_eq!(outcome.usage.completion_tokens, 1);
}

#[test]
fn interrupted_recovery_skips_a_text_completion() {
    let mut response = Response::Text(TextResponse::default());
    if let Response::Text(text) = &mut response {
        text.usage = TextUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
        };
    }
    let chunks = cut_stream();
    let changed = recover_interrupted_stream_output_tokens(
        &mut response,
        &chunks,
        "claude-test",
        &FixedCount(9),
    )
    .expect("a text completion is skipped");
    assert!(!changed);
    assert_eq!(
        usage_summary(&response).completion_tokens,
        1,
        "a text completion keeps its usage"
    );
}

#[test]
fn interrupted_recovery_tolerates_a_count_error() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let mut response = chat_response(&interrupted_spec());
    let chunks = cut_stream();
    let changed = recover_interrupted_stream_output_tokens(
        &mut response,
        &chunks,
        "claude-test",
        &FailingCount,
    )
    .expect("a count error is logged, not returned");
    assert!(!changed);
    assert_eq!(usage_summary(&response).completion_tokens, 1);
    assert!(log.contents().contains(
        "Could not re-tokenize interrupted stream output; keeping placeholder completion token count."
    ));
}

#[test]
fn interrupted_recovery_leaves_absent_details_absent() {
    let spec = ChatSpec {
        has_details: false,
        text_tokens: None,
        ..interrupted_spec()
    };
    let outcome = recover_with_fixed_count(&spec, &[cut_stream_bytes()], "claude-test", 5)
        .expect("a valid recovery");
    assert!(outcome.changed);
    assert_eq!(outcome.usage.completion_tokens, 5);
    assert_eq!(outcome.usage.text_tokens, None);
}

#[test]
fn interrupted_recovery_does_not_create_text_tokens_when_absent() {
    let spec = ChatSpec {
        has_details: true,
        text_tokens: None,
        ..interrupted_spec()
    };
    let outcome = recover_with_fixed_count(&spec, &[cut_stream_bytes()], "claude-test", 5)
        .expect("a valid recovery");
    assert!(outcome.changed);
    assert_eq!(outcome.usage.text_tokens, None);
}

#[test]
fn interrupted_recovery_counts_tool_call_json_with_the_logged_model() {
    let mut response = chat_response(&interrupted_spec());
    let Response::Chat(chat) = &mut response else {
        panic!("the spec builds a chat response");
    };
    chat.choices[0].message.content = Some("hello ".to_owned());
    chat.choices[0].message.function_call = Some(FunctionOutput {
        name: Some("f".to_owned()),
        arguments: "{\"y\":2}".to_owned(),
    });
    chat.choices[0].message.tool_calls = Some(vec![ToolOutput {
        id: "tool-1".to_owned(),
        kind: "function".to_owned(),
        function: Some(FunctionOutput {
            name: Some("lookup".to_owned()),
            arguments: "{\"x\":1}".to_owned(),
        }),
        custom: None,
        provider_specific_fields: None,
    }]);
    let counter = RecordingCount::new(9);
    let chunks = cut_stream();
    let changed =
        recover_interrupted_stream_output_tokens(&mut response, &chunks, "claude-logged", &counter)
            .expect("a valid recovery");
    assert!(changed);
    assert_eq!(
        counter.models(),
        vec!["claude-logged".to_owned()],
        "the recount uses the logged model"
    );
    let expected = concat!(
        "hello ",
        "{\"arguments\":\"{\\\"y\\\":2}\",\"name\":\"f\"}",
        "{\"function\":{\"arguments\":\"{\\\"x\\\":1}\",\"name\":\"lookup\"},\"id\":\"tool-1\",\"type\":\"function\"}",
    );
    assert_eq!(counter.texts(), vec![expected.to_owned()]);
}

#[test]
fn interrupted_recovery_saturates_total_tokens() {
    let spec = ChatSpec {
        prompt_tokens: i64::MAX,
        ..interrupted_spec()
    };
    let outcome = recover_with_fixed_count(&spec, &[cut_stream_bytes()], "claude-test", 5)
        .expect("a valid recovery");
    assert!(outcome.changed);
    assert_eq!(outcome.usage.total_tokens, i64::MAX);
}

#[test]
fn interrupted_clear_removes_cost_and_hidden_response_cost() {
    let spec = ChatSpec {
        cost: Some("0.00025".to_owned()),
        hidden_response_cost: Some("0.00025".to_owned()),
        ..interrupted_spec()
    };
    let before = usage_summary(&chat_response(&spec));
    assert_eq!(before.cost, Some(Decimal::new(25, 5)));
    assert_eq!(before.hidden_response_cost, Some(Decimal::new(25, 5)));
    let cleared = clear_cost_for_testing(&spec);
    assert!(cleared.cost_cleared);
    assert!(cleared.hidden_cost_cleared);
}

#[test]
fn interrupted_clear_ignores_a_text_completion() {
    let mut response = Response::Text(TextResponse::default());
    clear_placeholder_cost(&mut response);
    assert_eq!(usage_summary(&response).cost, None);
}

#[test]
fn interrupted_assemble_error_falls_back_with_the_warning() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let counter = FixedCount(5);
    let services = InterruptedStreamServices {
        costs: &cost,
        counter: &counter,
    };
    let call_log = logging_state(&[], &[]);
    let chunks = vec![chunk(ERROR)];
    let response = assemble_streaming_response(&chunks, &call_log, "claude-test", None, &services);
    assert!(response.is_none());
    let text = log.contents();
    assert!(text.contains("Anthropic passthrough: stream assembly raised (model=claude-test): "));
    assert!(text.contains("falling back to usage-only cost from raw SSE events."));
}

#[test]
fn interrupted_assemble_none_falls_back_silently() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let counter = FixedCount(5);
    let services = InterruptedStreamServices {
        costs: &cost,
        counter: &counter,
    };
    let call_log = logging_state(&[], &[]);
    let chunks = vec![chunk("event: ping\n\n")];
    let response = assemble_streaming_response(&chunks, &call_log, "claude-test", None, &services);
    assert!(response.is_none());
    let text = log.contents();
    assert!(!text.contains("stream assembly raised"));
    assert!(!text.contains("usage-only fallback failed"));
}

#[test]
fn interrupted_usage_only_builds_the_response_from_message_start_usage() {
    let response = usage_only_response_or_none(&[chunk(START)], "claude-test", None)
        .expect("message_start carries usage");
    let summary = usage_summary(&response);
    assert_eq!(summary.prompt_tokens, 10);
    assert_eq!(summary.completion_tokens, 1);
    let Response::Chat(chat) = &response else {
        panic!("the usage-only builder returns a chat response");
    };
    assert!(
        chat.created >= 1_700_000_000,
        "the usage-only response is stamped now"
    );
}

#[test]
fn interrupted_build_clears_only_after_a_recovery() {
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let counter = FixedCount(5);
    let services = InterruptedStreamServices {
        costs: &cost,
        counter: &counter,
    };
    let call_log = logging_state(&[], &[]);
    let body = request_body("claude-test");
    let chunks = cut_stream();
    let response =
        build_streaming_response_for_logging(&call_log, &body, &chunks, "claude-test", &services)
            .expect("a valid build")
            .expect("an assembled response");
    let Response::Chat(chat) = &response else {
        panic!("the stream assembles a chat response");
    };
    assert!(
        chat.created >= 1_700_000_000,
        "the chunk carries the wall-clock creation stamp"
    );
    let summary = usage_summary(&response);
    assert_eq!(
        summary.completion_tokens, 5,
        "the recount replaces the placeholder"
    );
    assert_eq!(summary.cost, None, "the placeholder cost is dropped");
    assert_eq!(summary.hidden_response_cost, None);
}

#[test]
fn interrupted_build_keeps_the_cost_of_a_completed_stream() {
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let counter = FixedCount(5);
    let services = InterruptedStreamServices {
        costs: &cost,
        counter: &counter,
    };
    let call_log = logging_state(&[], &[]);
    let body = request_body("claude-test");
    let chunks = completed_stream();
    let response =
        build_streaming_response_for_logging(&call_log, &body, &chunks, "claude-test", &services)
            .expect("a valid build")
            .expect("an assembled response");
    let summary = usage_summary(&response);
    assert_eq!(
        summary.completion_tokens, 5,
        "the stream reported its own count"
    );
    assert_eq!(
        summary.cost,
        Some(Decimal::new(25, 2)),
        "a completed stream keeps its cost"
    );
    assert_eq!(summary.hidden_response_cost, Some(Decimal::new(25, 2)));
}

#[test]
fn interrupted_build_passes_the_request_speed_to_the_usage() {
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let counter = FixedCount(5);
    let services = InterruptedStreamServices {
        costs: &cost,
        counter: &counter,
    };
    let call_log = logging_state(&[], &[]);
    let mut body = request_body("claude-test");
    body.insert("speed".to_owned(), Json::Text("fast".to_owned()));
    let chunks = cut_stream();
    let response =
        build_streaming_response_for_logging(&call_log, &body, &chunks, "claude-test", &services)
            .expect("a valid build")
            .expect("an assembled response");
    let Response::Chat(chat) = &response else {
        panic!("the stream assembles a chat response");
    };
    assert_eq!(chat.usage.speed.as_deref(), Some("fast"));
}

#[test]
fn interrupted_build_skips_the_hidden_cost_for_an_xai_provider() {
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let counter = FixedCount(5);
    let services = InterruptedStreamServices {
        costs: &cost,
        counter: &counter,
    };
    let call_log = logging_state(
        &[],
        &[("custom_llm_provider", Json::Text("xai".to_owned()))],
    );
    let body = request_body("claude-test");
    let chunks = completed_stream();
    let response =
        build_streaming_response_for_logging(&call_log, &body, &chunks, "claude-test", &services)
            .expect("a valid build")
            .expect("an assembled response");
    let summary = usage_summary(&response);
    assert_eq!(
        summary.cost,
        Some(Decimal::new(25, 2)),
        "the xai stream keeps its usage cost"
    );
    assert_eq!(
        summary.hidden_response_cost, None,
        "xai prices the reported cost itself"
    );
}

#[test]
fn interrupted_build_returns_none_without_a_reconstructable_stream() {
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let counter = FixedCount(5);
    let services = InterruptedStreamServices {
        costs: &cost,
        counter: &counter,
    };
    let call_log = logging_state(&[], &[]);
    let body = request_body("claude-test");
    let chunks = vec![chunk("event: ping\n\n")];
    let response =
        build_streaming_response_for_logging(&call_log, &body, &chunks, "claude-test", &services)
            .expect("a valid build");
    assert!(response.is_none());
}

#[test]
fn interrupted_record_does_nothing_for_empty_chunks() {
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let counter = FixedCount(5);
    let services = InterruptedStreamServices {
        costs: &cost,
        counter: &counter,
    };
    let mut call_log = logging_state(&[], &[]);
    let body = request_body("claude-test");
    record_partial_usage_for_failure(&mut call_log, &body, &[], &services).expect("an empty call");
    assert!(
        !call_log
            .model_call_details
            .contains_key("combined_usage_object")
    );
    assert!(!call_log.model_call_details.contains_key("response_cost"));
}

#[test]
fn interrupted_record_bills_zero_cost_when_costing_fails() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let cost = FakeCost::unpriced();
    let counter = FixedCount(5);
    let services = InterruptedStreamServices {
        costs: &cost,
        counter: &counter,
    };
    let mut call_log = logging_state(&[], &[]);
    let body = request_body("claude-test");
    let chunks = cut_stream();
    record_partial_usage_for_failure(&mut call_log, &body, &chunks, &services)
        .expect("a recorded failure");
    let recorded = recorded_usage(&call_log);
    assert_eq!(number(recorded.get("completion_tokens")), Some(5));
    assert_eq!(number(recorded.get("total_tokens")), Some(15));
    assert_eq!(recorded_cost(&call_log), Some(0.0_f64));
    assert!(log.contents().contains(
        "Anthropic passthrough: could not cost the partial usage of an interrupted stream (model=claude-test): "
    ));
}

#[test]
fn interrupted_record_bills_the_partial_stream_cost() {
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let counter = FixedCount(5);
    let services = InterruptedStreamServices {
        costs: &cost,
        counter: &counter,
    };
    let mut call_log = logging_state(&[], &[]);
    let body = request_body("claude-test");
    let chunks = cut_stream();
    record_partial_usage_for_failure(&mut call_log, &body, &chunks, &services)
        .expect("a recorded failure");
    let recorded = recorded_usage(&call_log);
    assert_eq!(number(recorded.get("prompt_tokens")), Some(10));
    assert_eq!(number(recorded.get("completion_tokens")), Some(5));
    assert_eq!(recorded_cost(&call_log), Some(0.25_f64));
}

#[test]
fn interrupted_usage_copies_every_detail_count_into_the_recorded_usage() {
    let recorded = usage_of(&detail_counts_response());
    assert_eq!(recorded.prompt_tokens, 10);
    assert_eq!(recorded.completion_tokens, 5);
    assert_eq!(recorded.total_tokens, 15);
    assert_eq!(recorded.cost, Some(Decimal::new(25, 2)));
    assert_prompt_details(
        &recorded
            .prompt_tokens_details
            .expect("the prompt details are recorded"),
    );
    assert_completion_details(
        &recorded
            .completion_tokens_details
            .expect("the completion details are recorded"),
    );
    assert_server_tool_use(
        &recorded
            .server_tool_use
            .expect("the server tool use is recorded"),
    );
}

#[test]
fn interrupted_record_keeps_the_cache_counts_geo_and_speed_in_the_usage_object() {
    let mut call_log = logging_state(&[], &[]);
    let usage = usage_of(&detail_counts_response());
    record_logged_usage(&mut call_log, &usage, Decimal::ZERO);
    let recorded = recorded_usage(&call_log);
    assert_count(recorded, "cache_read_input_tokens", 3);
    assert_count(recorded, "cache_creation_input_tokens", 2);
    assert_eq!(
        recorded.get("inference_geo"),
        Some(&Json::Text("us".to_owned()))
    );
    assert_eq!(recorded.get("speed"), Some(&Json::Text("fast".to_owned())));
}

fn assert_count(fields: &IndexMap<String, Json>, name: &str, expected: i64) {
    assert_eq!(
        fields.get(name),
        Some(&Json::Number(Number::from(expected))),
        "{name}"
    );
}

fn assert_prompt_details(prompt: &IndexMap<String, Json>) {
    assert_count(prompt, "cached_tokens", 7);
    assert_count(prompt, "web_search_requests", 1);
    assert_count(prompt, "google_maps_grounding_requests", 2);
    assert_count(prompt, "cache_write_tokens", 3);
    assert_count(prompt, "cache_creation_tokens", 4);
    assert_cache_creation_details(prompt);
    assert_count(prompt, "custom_prompt", 9);
}

fn assert_cache_creation_details(prompt: &IndexMap<String, Json>) {
    let mut nested = IndexMap::new();
    nested.insert(
        "ephemeral_5m_input_tokens".to_owned(),
        Json::Number(Number::from(5_i64)),
    );
    nested.insert(
        "ephemeral_1h_input_tokens".to_owned(),
        Json::Number(Number::from(6_i64)),
    );
    assert_eq!(
        prompt.get("cache_creation_token_details"),
        Some(&Json::Object(nested))
    );
}

fn assert_completion_details(completion: &IndexMap<String, Json>) {
    assert_count(completion, "reasoning_tokens", 1);
    assert_count(completion, "text_tokens", 4);
    assert_count(completion, "image_tokens", 0);
    assert!(completion.get("audio_tokens").is_none());
    assert_eq!(completion.get("custom_completion"), Some(&Json::Bool(true)));
}

fn assert_server_tool_use(server: &IndexMap<String, Json>) {
    assert_count(server, "web_search_requests", 1);
    assert_count(server, "tool_search_requests", 2);
    assert_count(server, "browser_open_requests", 3);
}

#[test]
fn interrupted_record_does_nothing_without_a_reconstructable_stream() {
    let cost = FakeCost::priced(Decimal::new(25, 2));
    let counter = FixedCount(5);
    let services = InterruptedStreamServices {
        costs: &cost,
        counter: &counter,
    };
    let mut call_log = logging_state(&[], &[]);
    let body = request_body("claude-test");
    let chunks = vec![chunk("event: ping\n\n")];
    record_partial_usage_for_failure(&mut call_log, &body, &chunks, &services)
        .expect("nothing to record");
    assert!(
        !call_log
            .model_call_details
            .contains_key("combined_usage_object")
    );
}

#[rstest]
fn replays_the_stream_interrupted_corpus(
    #[files("fuzz/corpus/stream_interrupted/*")] path: PathBuf,
) {
    let bytes = read(path).expect("a corpus seed");
    let _decided = stream_was_interrupted_bytes(&[bytes]);
}

fn timestamp() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").expect("a valid timestamp")
}

fn logging_state(litellm_params: &[(&str, Json)], details: &[(&str, Json)]) -> LoggingState {
    let redactor = Redactor::new(16).expect("a valid redactor");
    let mut state = LoggingState::new(
        LoggingInit {
            model: String::new(),
            call_type: "pass_through_endpoint".to_owned(),
            start_time: timestamp(),
            litellm_call_id: "call-1".to_owned(),
            function_id: "function-1".to_owned(),
            messages: None,
            stream: Some(false),
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: params(litellm_params),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("a valid logging state");
    for (name, value) in details {
        state
            .model_call_details
            .insert((*name).to_owned(), value.clone());
    }
    state
}

fn params(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    let mut map = IndexMap::new();
    for (name, value) in entries {
        map.insert((*name).to_owned(), value.clone());
    }
    map
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture_logs(level: tracing::Level) -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
