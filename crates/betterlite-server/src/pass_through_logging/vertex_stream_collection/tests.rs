#![cfg(test)]

use std::sync::Arc;

use betterlite_core::routes::CallType;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::types::CollectedChunksHandler;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use crate::pass_through_logging::gemini_passthrough_logging::counter::CollectedCounter;
use crate::pass_through_logging::gemini_passthrough_logging::tests::support::{
    FakeCost, as_text, capture_logs, collected, detail, field, integer, json, kwarg, logged_model,
    number_text, optional_param, result_json, text,
};

use super::{VertexStreamCollector, build};

const VERTEX_GENERATE_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/us-central1/publishers/google/models/gemini-2.0-flash:generateContent";
const VERTEX_STREAM_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/us-central1/publishers/google/models/gemini-2.0-flash:streamGenerateContent";
const VERTEX_RAW_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/us-central1/publishers/google/models/claude-3:rawPredict";
const VERTEX_STREAM_RAW_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/us-central1/publishers/google/models/claude-3:streamRawPredict";
const VERTEX_PREDICT_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/us-central1/publishers/google/models/m:predict";
const GEMINI_GENERATE_ROUTE: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent";

const CHUNK: &str = r#"{"responseId":"r1","modelVersion":"gemini-2.0-flash","candidates":[{"content":{"parts":[{"text":"hi"}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5}}"#;
const NO_MODEL_CHUNK: &str = r#"{"responseId":"r1","candidates":[{"content":{"parts":[{"text":"hi"}],"role":"model"},"finishReason":"STOP","index":0}]}"#;

const MESSAGE_START: &str = r#"data: {"type":"message_start","message":{"id":"msg_1","role":"assistant","model":"claude-3","usage":{"input_tokens":10,"output_tokens":1}}}"#;
const TEXT_DELTA: &str =
    r#"data:{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" hi"}}"#;
const MESSAGE_DELTA: &str =
    r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#;
const THINKING_DELTA: &str = r#"data:{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Hello world"}}"#;
const HELLO_DELTA: &str =
    r#"data:{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#;
const NO_USAGE_STOP: &str = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#;

const VERTEX_MISSING_RESPONSE: &str =
    "Unable to build complete streaming response for Vertex passthrough endpoint, not logging...";

#[test]
fn build_generate_route_uses_the_decoder() {
    let lines = [CHUNK.to_owned()];
    let body = build::vertex_response(
        &lines,
        VERTEX_GENERATE_ROUTE,
        "gemini-2.0-flash",
        0,
        Some(&bundled_counter()),
    )
    .expect("the chunk builds a response")
    .expect("a response is built");
    assert_eq!(text(&body, "model"), "gemini-2.0-flash");
    assert_eq!(content(&body), "hi");
    assert_eq!(integer(field(&body, "usage"), "total_tokens"), 5);
}

#[test]
fn build_stream_generate_route_uses_the_decoder() {
    let lines = [NO_MODEL_CHUNK.to_owned()];
    let body = build::vertex_response(
        &lines,
        VERTEX_STREAM_ROUTE,
        "url-model",
        0,
        Some(&bundled_counter()),
    )
    .expect("the chunk builds a response")
    .expect("a response is built");
    assert_eq!(field(&body, "model"), &Json::Null);
    assert_eq!(content(&body), "hi");
}

#[test]
fn build_raw_predict_strips_both_prefixes_and_skips_bad_lines() {
    let lines = [
        MESSAGE_START.to_owned(),
        TEXT_DELTA.to_owned(),
        "not json".to_owned(),
        "[1,2]".to_owned(),
        "data: ".to_owned(),
        MESSAGE_DELTA.to_owned(),
    ];
    let body = build::vertex_response(
        &lines,
        VERTEX_RAW_ROUTE,
        "claude-3",
        0,
        Some(&bundled_counter()),
    )
    .expect("the events build a response")
    .expect("a response is built");
    assert_eq!(content(&body), " hi");
    assert_eq!(
        text(&choice(&body), "finish_reason"),
        "stop",
        "the message delta closes the reply"
    );
}

#[test]
fn build_stream_raw_predict_uses_the_parser() {
    let lines = [MESSAGE_START.to_owned(), TEXT_DELTA.to_owned()];
    let body = build::vertex_response(
        &lines,
        VERTEX_STREAM_RAW_ROUTE,
        "claude-3",
        0,
        Some(&bundled_counter()),
    )
    .expect("the events build a response")
    .expect("a response is built");
    assert_eq!(content(&body), " hi");
}

#[test]
fn build_unknown_route_returns_no_response() {
    let lines = [CHUNK.to_owned()];
    assert!(
        build::vertex_response(
            &lines,
            VERTEX_PREDICT_ROUTE,
            "m",
            0,
            Some(&bundled_counter())
        )
        .expect("an unknown route is not an error")
        .is_none()
    );
}

#[test]
fn build_raw_predict_without_usable_lines_returns_no_response() {
    let lines = ["not json".to_owned(), "data: ".to_owned()];
    assert!(
        build::vertex_response(&lines, VERTEX_RAW_ROUTE, "m", 0, Some(&bundled_counter()))
            .expect("no usable line is not an error")
            .is_none()
    );
}

#[test]
fn chunks_store_the_location_before_the_response_work() {
    let fixture = collected(VERTEX_PREDICT_ROUTE, &[CHUNK], None);
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let collector = VertexStreamCollector::new(cost);
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let outcome = collector
        .handle(&fixture.call())
        .expect("a predict route is not an error");
    assert_eq!(outcome.result, None);
    assert_eq!(outcome.kwargs, PassThroughLoggingKwargs::default());
    assert_eq!(
        optional_param(&fixture.logging_obj, "vertex_location"),
        Some(Json::Text("us-central1".to_owned()))
    );
    assert!(logs.contents().contains(VERTEX_MISSING_RESPONSE));
}

#[test]
fn chunks_vertex_host_uses_the_vertex_ai_provider() {
    let fixture = collected(VERTEX_GENERATE_ROUTE, &[CHUNK], None);
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let collector = VertexStreamCollector::new(Arc::clone(&cost));
    let outcome = collector
        .handle(&fixture.call())
        .expect("the chunks build a response");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "id"), "call-1");
    assert_eq!(text(&result, "model"), "gemini-2.0-flash");
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "1.25");
    assert_eq!(as_text(kwarg(&outcome, "model")), "gemini-2.0-flash");
    assert_eq!(outcome.kwargs.entries.len(), 2);
    assert_eq!(logged_model(&fixture.logging_obj), "gemini-2.0-flash");
    assert_eq!(
        detail(&fixture.logging_obj, "model"),
        Some(Json::Text("gemini-2.0-flash".to_owned()))
    );
    assert_eq!(
        detail(&fixture.logging_obj, "custom_llm_provider"),
        Some(Json::Text("vertex_ai".to_owned()))
    );
    assert_eq!(detail(&fixture.logging_obj, "response_cost"), None);
    assert_eq!(
        optional_param(&fixture.logging_obj, "vertex_location"),
        Some(Json::Text("us-central1".to_owned()))
    );
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].provider, "vertex_ai");
    assert_eq!(calls[0].call_type, CallType::Completion);
    assert_eq!(calls[0].response_model.as_deref(), Some("gemini-2.0-flash"));
    assert_eq!(calls[0].usage, Some((3, 2, 5)));
    assert_eq!(calls[0].vertex_location.as_deref(), Some("us-central1"));
}

#[test]
fn chunks_gemini_host_uses_the_gemini_provider() {
    let fixture = collected(GEMINI_GENERATE_ROUTE, &[CHUNK], None);
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let collector = VertexStreamCollector::new(Arc::clone(&cost));
    collector
        .handle(&fixture.call())
        .expect("the chunks build a response");
    assert_eq!(
        optional_param(&fixture.logging_obj, "vertex_location"),
        None
    );
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].provider, "gemini");
    assert_eq!(calls[0].vertex_location, None);
}

#[test]
fn chunks_use_the_given_model_when_the_stream_names_none() {
    let fixture = collected(
        VERTEX_GENERATE_ROUTE
            .replace("gemini-2.0-flash", "url-model")
            .as_str(),
        &[NO_MODEL_CHUNK],
        Some("given-model"),
    );
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let collector = VertexStreamCollector::new(Arc::clone(&cost));
    let outcome = collector
        .handle(&fixture.call())
        .expect("the chunks build a response");
    let result = result_json(&outcome);
    assert_eq!(field(&result, "model"), &Json::Null);
    assert_eq!(as_text(kwarg(&outcome, "model")), "given-model");
    assert_eq!(cost.calls()[0].model, "given-model");
    assert_eq!(cost.calls()[0].response_model, None);
}

#[test]
fn chunks_empty_model_falls_back_to_the_url_model() {
    let fixture = collected(
        VERTEX_GENERATE_ROUTE
            .replace("gemini-2.0-flash", "url-model")
            .as_str(),
        &[NO_MODEL_CHUNK],
        Some(""),
    );
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let collector = VertexStreamCollector::new(cost);
    let outcome = collector
        .handle(&fixture.call())
        .expect("the chunks build a response");
    assert_eq!(field(&result_json(&outcome), "model"), &Json::Null);
    assert_eq!(as_text(kwarg(&outcome, "model")), "url-model");
    assert_eq!(logged_model(&fixture.logging_obj), "url-model");
}

#[test]
fn chunks_raw_predict_without_usable_lines_returns_the_error_text() {
    let fixture = collected(VERTEX_STREAM_RAW_ROUTE, &["not json"], None);
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let collector = VertexStreamCollector::new(cost);
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let outcome = collector
        .handle(&fixture.call())
        .expect("no usable event is not an error");
    assert_eq!(outcome.result, None);
    assert_eq!(outcome.kwargs, PassThroughLoggingKwargs::default());
    assert!(logs.contents().contains(VERTEX_MISSING_RESPONSE));
}

#[test]
fn chunks_raw_predict_builds_and_prices_the_events() {
    let fixture = collected(
        VERTEX_RAW_ROUTE,
        &[MESSAGE_START, TEXT_DELTA, MESSAGE_DELTA],
        None,
    );
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let collector = VertexStreamCollector::new(Arc::clone(&cost));
    let outcome = collector
        .handle(&fixture.call())
        .expect("the events build a response");
    assert_eq!(content(&result_json(&outcome)), " hi");
    assert_eq!(as_text(kwarg(&outcome, "model")), "claude-3");
    assert_eq!(cost.calls()[0].provider, "vertex_ai");
}

#[test]
fn chunks_raw_predict_without_usage_counts_the_text_and_reasoning() {
    let fixture = collected(
        VERTEX_RAW_ROUTE,
        &[THINKING_DELTA, HELLO_DELTA, NO_USAGE_STOP],
        None,
    );
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let collector = VertexStreamCollector::new(Arc::clone(&cost));
    let outcome = collector
        .handle(&fixture.call())
        .expect("the events build a response");
    let usage = field(&result_json(&outcome), "usage");
    assert_eq!(integer(usage, "prompt_tokens"), 0);
    assert_eq!(integer(usage, "completion_tokens"), 3);
    assert_eq!(integer(usage, "total_tokens"), 3);
    let details = field(usage, "completion_tokens_details");
    assert_eq!(integer(details, "reasoning_tokens"), 2);
    assert_eq!(integer(details, "text_tokens"), 1);
}

#[test]
fn build_cost_record_reads_only_a_non_empty_body_model() {
    assert_eq!(
        build::body_model(&json(r#"{"model":"gemini-2.0-flash"}"#)),
        Some("gemini-2.0-flash".to_owned())
    );
    assert_eq!(build::body_model(&json(r#"{"model":""}"#)), None);
    assert_eq!(build::body_model(&json("[]")), None);
}

fn bundled_counter() -> CollectedCounter {
    let built = CollectedCounter::bundled();
    built.expect("the bundled cl100k vocabulary loads")
}

fn choice(body: &Json) -> &Json {
    let Json::List(rows) = field(body, "choices") else {
        panic!("the choices are a list");
    };
    rows.first().expect("one choice")
}

fn content(body: &Json) -> String {
    let message = field(choice(body), "message");
    match field(message, "content") {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("the message carries text")
        }
    }
}
