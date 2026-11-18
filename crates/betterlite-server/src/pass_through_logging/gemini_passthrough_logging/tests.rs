#![cfg(test)]

use std::sync::Arc;

use betterlite_core::routes::CallType;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::types::{CollectedChunksHandler, HandlerError};
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::UsageNumber;
use betterlite_redact::structured::Payload;
use betterlite_streaming::stream_chunk_shaping::records::JsonObject;
use betterlite_streaming::stream_usage_assembly::wire::Usage as WireUsage;
use betterlite_streaming::streamed_response_assembly::cost::{CostInput, StreamCostCalculator};
use betterlite_vertex::vertex_relay_metadata::types::{CustomLlmProvider, ResponseLoggingKwargs};
use rstest::rstest;
use rust_decimal::Decimal;

use super::GeminiPassthroughError;
use super::assembly;
use super::cost::PassthroughCostError;
use super::payload::create_gemini_response_logging_payload;
use super::relay::{RelayLogging, relay_kwargs};
use super::reply;
use support::{
    FakeCost, as_text, capture_logs, collected, detail, field, fixture, gemini_input, integer,
    json, kwarg, kwargs, logged_model, logged_provider, logger, number_text, result_json, text,
};

pub(crate) mod support;

const GENERATE_ROUTE: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent";
const STREAM_ROUTE: &str = "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:streamGenerateContent";
const VIDEO_ROUTE: &str = "https://generativelanguage.googleapis.com/v1beta/models/veo-3.0-generate-001:predictLongRunning";
const INTERACTIONS_ROUTE: &str =
    "https://generativelanguage.googleapis.com/v1beta/interactions?alt=generateContent";
const PREDICT_ROUTE: &str = "https://generativelanguage.googleapis.com/v1beta/models/m:predict";

const GENERATE_BODY: &str = r#"{"candidates":[{"content":{"parts":[{"text":"hello"}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5},"modelVersion":"gemini-2.0-flash"}"#;
const INTERACTIONS_BODY: &str =
    r#"{"model":"response-model","usage":{"total_input_tokens":3,"total_output_tokens":2}}"#;
const VIDEO_BODY: &str = r#"{"name":"operations/generate_1234567890"}"#;
const NO_MODEL_BODY: &str = r#"{"candidates":[{"content":{"parts":[{"text":"hello"}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5}}"#;
const DETAILS_BODY: &str = r#"{"candidates":[{"content":{"parts":[{"text":"hello"}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5,"promptTokensDetails":[{"modality":"TEXT","tokenCount":3}]},"modelVersion":"gemini-2.0-flash"}"#;
const USAGE_BODY: &str = r#"{"model":"response-model","usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#;

const MISSING_RESPONSE: &str =
    "Unable to build complete streaming response for Gemini passthrough endpoint, not logging...";

#[test]
fn route_interactions_never_reaches_the_generate_content_decode() {
    let mut fixture = fixture(INTERACTIONS_ROUTE, INTERACTIONS_BODY);
    fixture.request_body.insert(
        "model".to_owned(),
        serde_json::Value::String("request-model".to_owned()),
    );
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let outcome = logger(Arc::clone(&cost))
        .gemini_passthrough_handler(gemini_input(&fixture))
        .expect("the interactions reply is logged");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "id"), "call-1");
    assert_eq!(text(&result, "model"), "response-model");
    let usage = field(&result, "usage");
    assert_eq!(integer(usage, "prompt_tokens"), 3);
    assert_eq!(integer(usage, "completion_tokens"), 2);
    assert_eq!(integer(usage, "total_tokens"), 5);
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "1.25");
    assert_eq!(as_text(kwarg(&outcome, "custom_llm_provider")), "gemini");
    assert_eq!(
        logged_provider(&fixture.logging_obj),
        Some("gemini".to_owned())
    );
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].kind, "completion");
    assert_eq!(calls[0].model, "response-model");
}

#[test]
fn route_interactions_without_a_usage_object_keeps_the_input_kwargs() {
    let fixture = fixture(INTERACTIONS_ROUTE, r#"{"model":"response-model"}"#);
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let mut input = gemini_input(&fixture);
    input.kwargs = kwargs(&[("request_id", Json::Text("r1".to_owned()))]);
    let outcome = logger(Arc::clone(&cost))
        .gemini_passthrough_handler(input)
        .expect("an interactions reply without usage is not an error");
    assert_eq!(outcome.result, None);
    assert_eq!(
        outcome.kwargs,
        kwargs(&[("request_id", Json::Text("r1".to_owned()))])
    );
    assert_eq!(logged_provider(&fixture.logging_obj), None);
    assert!(cost.calls().is_empty());
}

#[test]
fn route_predict_long_running_builds_and_prices_the_video() {
    let fixture = fixture(VIDEO_ROUTE, VIDEO_BODY);
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let outcome = logger(Arc::clone(&cost))
        .gemini_passthrough_handler(gemini_input(&fixture))
        .expect("the video operation is logged");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "object"), "video");
    assert_eq!(text(&result, "status"), "processing");
    assert_eq!(text(&result, "model"), "veo-3.0-generate-001");
    let Json::Object(fields) = &result else {
        panic!("the video body is an object");
    };
    assert!(!fields.contains_key("_hidden_params"));
    assert!(!fields.contains_key("hidden_params"));
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "1.25");
    assert_eq!(as_text(kwarg(&outcome, "model")), "veo-3.0-generate-001");
    assert_eq!(as_text(kwarg(&outcome, "custom_llm_provider")), "gemini");
    assert_eq!(logged_model(&fixture.logging_obj), "veo-3.0-generate-001");
    assert_eq!(
        logged_provider(&fixture.logging_obj),
        Some("gemini".to_owned())
    );
    assert_eq!(
        detail(&fixture.logging_obj, "model"),
        Some(Json::Text("veo-3.0-generate-001".to_owned()))
    );
    assert_eq!(
        detail(&fixture.logging_obj, "custom_llm_provider"),
        Some(Json::Text("gemini".to_owned()))
    );
    let recorded =
        detail(&fixture.logging_obj, "response_cost").expect("the details carry the cost");
    assert_eq!(number_text(&recorded), "1.25");
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "veo-3.0-generate-001");
    assert_eq!(calls[0].provider, "gemini");
    assert_eq!(calls[0].call_type, CallType::CreateVideo);
    assert_eq!(calls[0].kind, "video");
    assert_eq!(
        calls[0].response_model.as_deref(),
        Some("veo-3.0-generate-001")
    );
}

#[test]
fn route_generate_content_decodes_and_prices_the_response() {
    let fixture = fixture(GENERATE_ROUTE, GENERATE_BODY);
    let cost = FakeCost::priced(Decimal::new(25, 3));
    let outcome = logger(Arc::clone(&cost))
        .gemini_passthrough_handler(gemini_input(&fixture))
        .expect("the generateContent reply is logged");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "id"), "call-1");
    assert_eq!(text(&result, "object"), "chat.completion");
    assert_eq!(text(&result, "model"), "gemini-2.0-flash");
    let choices = choices(&result);
    assert_eq!(text(choices, "finish_reason"), "stop");
    assert_eq!(integer(choices, "index"), 0);
    let message = field(choices, "message");
    assert_eq!(text(message, "content"), "hello");
    assert_eq!(text(message, "role"), "assistant");
    let usage = field(&result, "usage");
    assert_eq!(integer(usage, "prompt_tokens"), 3);
    assert_eq!(integer(usage, "completion_tokens"), 2);
    assert_eq!(integer(usage, "total_tokens"), 5);
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "0.025");
    assert_eq!(as_text(kwarg(&outcome, "model")), "gemini-2.0-flash");
    assert_eq!(as_text(kwarg(&outcome, "custom_llm_provider")), "gemini");
    assert_eq!(logged_model(&fixture.logging_obj), "gemini-2.0-flash");
    assert_eq!(
        detail(&fixture.logging_obj, "custom_llm_provider"),
        Some(Json::Text("gemini".to_owned()))
    );
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "gemini-2.0-flash");
    assert_eq!(calls[0].provider, "gemini");
    assert_eq!(calls[0].call_type, CallType::Completion);
    assert_eq!(calls[0].kind, "completion");
    assert_eq!(calls[0].response_model.as_deref(), Some("gemini-2.0-flash"));
    assert_eq!(calls[0].usage, Some((3, 2, 5)));
}

#[test]
fn route_stream_generate_content_alone_is_not_the_generate_content_branch() {
    let fixture = fixture(
        STREAM_ROUTE,
        r#"{"candidates":[{"content":{"parts":[{"text":"hi"}]},"finishReason":"STOP"}]}"#,
    );
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let mut input = gemini_input(&fixture);
    input.kwargs = kwargs(&[("request_id", Json::Text("r1".to_owned()))]);
    let outcome = logger(Arc::clone(&cost))
        .gemini_passthrough_handler(input)
        .expect("the stream route is not a non-streaming branch");
    assert_eq!(outcome.result, None);
    assert_eq!(
        outcome.kwargs,
        kwargs(&[("request_id", Json::Text("r1".to_owned()))])
    );
    assert!(cost.calls().is_empty());
}

#[test]
fn route_unmatched_route_returns_the_input_kwargs() {
    let fixture = fixture(PREDICT_ROUTE, GENERATE_BODY);
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let mut input = gemini_input(&fixture);
    input.kwargs = kwargs(&[("request_id", Json::Text("r1".to_owned()))]);
    let outcome = logger(Arc::clone(&cost))
        .gemini_passthrough_handler(input)
        .expect("a predict route is not a Gemini branch");
    assert_eq!(outcome.result, None);
    assert_eq!(
        outcome.kwargs,
        kwargs(&[("request_id", Json::Text("r1".to_owned()))])
    );
    assert!(cost.calls().is_empty());
}

#[test]
fn route_decode_error_is_returned() {
    let fixture = fixture(GENERATE_ROUTE, "not json");
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let error = logger(Arc::clone(&cost))
        .gemini_passthrough_handler(gemini_input(&fixture))
        .expect_err("an undecodable reply is an error");
    assert!(matches!(error, GeminiPassthroughError::Decode { .. }));
    assert!(cost.calls().is_empty());
}

#[test]
fn route_cost_error_is_returned() {
    let fixture = fixture(VIDEO_ROUTE, VIDEO_BODY);
    let cost = FakeCost::unpriced();
    let error = logger(Arc::clone(&cost))
        .gemini_passthrough_handler(gemini_input(&fixture))
        .expect_err("an unresolved video price is an error");
    match error {
        GeminiPassthroughError::Cost { model, source } => {
            assert_eq!(model, "veo-3.0-generate-001");
            assert!(matches!(source, PassthroughCostError::Unavailable { .. }));
        }
        other => panic!("the error is a cost failure, not {other:?}"),
    }
    assert_eq!(logged_provider(&fixture.logging_obj), None);
}

#[test]
fn payload_prices_with_the_literal_gemini_provider() {
    let logging_obj = support::logging_object();
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let mut response = json(USAGE_BODY);
    create_gemini_response_logging_payload(
        cost.as_ref(),
        &mut response,
        "request-model",
        PassThroughLoggingKwargs::default(),
        &logging_obj,
        "vertex_ai",
    )
    .expect("the payload is built");
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "request-model");
    assert_eq!(calls[0].provider, "gemini");
    assert_eq!(calls[0].call_type, CallType::Completion);
    assert_eq!(calls[0].response_model.as_deref(), Some("response-model"));
    assert_eq!(calls[0].usage, Some((1, 2, 3)));
}

#[test]
fn payload_sets_the_id_model_and_call_details() {
    let logging_obj = support::logging_object();
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let mut response = json(USAGE_BODY);
    let built = create_gemini_response_logging_payload(
        cost.as_ref(),
        &mut response,
        "request-model",
        PassThroughLoggingKwargs::default(),
        &logging_obj,
        "vertex_ai",
    )
    .expect("the payload is built");
    assert_eq!(text(&response, "id"), "call-1");
    assert_eq!(
        as_text(built.entries.get("model").expect("a model")),
        "request-model"
    );
    assert_eq!(
        as_text(
            built
                .entries
                .get("custom_llm_provider")
                .expect("a provider")
        ),
        "vertex_ai"
    );
    assert_eq!(
        number_text(built.entries.get("response_cost").expect("a cost")),
        "1.25"
    );
    assert_eq!(logged_model(&logging_obj), "response-model");
    assert_eq!(
        detail(&logging_obj, "model"),
        Some(Json::Text("response-model".to_owned()))
    );
    assert_eq!(
        detail(&logging_obj, "custom_llm_provider"),
        Some(Json::Text("vertex_ai".to_owned()))
    );
    let recorded = detail(&logging_obj, "response_cost").expect("the details carry the cost");
    assert_eq!(number_text(&recorded), "1.25");
}

#[rstest]
#[case::absent(r#"{"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#)]
#[case::empty(r#"{"model":"","usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#)]
fn payload_falls_back_to_the_argument_model(#[case] response: &str) {
    let logging_obj = support::logging_object();
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let mut body = json(response);
    create_gemini_response_logging_payload(
        cost.as_ref(),
        &mut body,
        "request-model",
        PassThroughLoggingKwargs::default(),
        &logging_obj,
        "gemini",
    )
    .expect("the payload is built");
    assert_eq!(logged_model(&logging_obj), "request-model");
    assert_eq!(
        detail(&logging_obj, "model"),
        Some(Json::Text("request-model".to_owned()))
    );
}

#[test]
fn payload_keeps_the_caller_kwargs() {
    let logging_obj = support::logging_object();
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let mut response = json(USAGE_BODY);
    let built = create_gemini_response_logging_payload(
        cost.as_ref(),
        &mut response,
        "request-model",
        kwargs(&[("request_id", Json::Text("r1".to_owned()))]),
        &logging_obj,
        "gemini",
    )
    .expect("the payload is built");
    assert_eq!(
        built.entries.get("request_id"),
        Some(&Json::Text("r1".to_owned()))
    );
    assert_eq!(built.entries.len(), 4);
}

#[test]
fn payload_returns_a_cost_error() {
    let logging_obj = support::logging_object();
    let cost = FakeCost::unpriced();
    let mut response = json(USAGE_BODY);
    let error = create_gemini_response_logging_payload(
        cost.as_ref(),
        &mut response,
        "request-model",
        PassThroughLoggingKwargs::default(),
        &logging_obj,
        "gemini",
    )
    .expect_err("an unresolved price is an error");
    assert!(matches!(error, GeminiPassthroughError::Cost { .. }));
    assert_eq!(logged_model(&logging_obj), "");
}

#[test]
fn chunks_build_a_response_from_sse_lines() {
    let fixture = collected(
        STREAM_ROUTE,
        &[
            r#"data: {"responseId":"r1","modelVersion":"gemini-2.0-flash","candidates":[{"content":{"parts":[{"text":"he"}],"role":"model"},"index":0}]}"#,
            "",
            r#"data: {"responseId":"r1","modelVersion":"gemini-2.0-flash","candidates":[{"content":{"parts":[{"text":"llo"}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5}}"#,
        ],
        None,
    );
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let outcome = logger(Arc::clone(&cost))
        .handle(&fixture.call())
        .expect("the SSE lines build a response");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "id"), "call-1");
    assert_eq!(text(&result, "model"), "gemini-2.0-flash");
    let choices = choices(&result);
    assert_eq!(text(field(choices, "message"), "content"), "hello");
    assert_eq!(text(choices, "finish_reason"), "stop");
    let usage = field(&result, "usage");
    assert_eq!(integer(usage, "prompt_tokens"), 3);
    assert_eq!(integer(usage, "completion_tokens"), 2);
    assert_eq!(integer(usage, "total_tokens"), 5);
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "1.25");
    assert_eq!(as_text(kwarg(&outcome, "model")), "gemini-2.0-flash");
    assert_eq!(as_text(kwarg(&outcome, "custom_llm_provider")), "gemini");
    assert_eq!(logged_model(&fixture.logging_obj), "gemini-2.0-flash");
    assert_eq!(logged_provider(&fixture.logging_obj), None);
    assert_eq!(cost.calls()[0].usage, Some((3, 2, 5)));
}

#[test]
fn chunks_accumulated_mode_builds_and_drops_the_partial_tail() {
    let fixture = collected(
        STREAM_ROUTE,
        &[
            r#"{"responseId":"r2","modelVersion":"gemini-2.0-flash","candidates":[{"content":{"parts":[{"text":"he"#,
            r#"llo"}],"role":"model"},"index":0}]}"#,
            r#"{"responseId":"r2","candidates":[{"content":{"parts":[{"text":"tail"#,
        ],
        None,
    );
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let outcome = logger(Arc::clone(&cost))
        .handle(&fixture.call())
        .expect("the split object builds a response");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "model"), "gemini-2.0-flash");
    let choice = choices(&result);
    assert_eq!(text(field(choice, "message"), "content"), "hello");
    assert_eq!(cost.calls().len(), 1);
}

#[test]
fn chunks_use_the_given_model_when_the_stream_names_none() {
    let fixture = collected(
        STREAM_ROUTE
            .replace("gemini-2.0-flash", "url-model")
            .as_str(),
        &[
            r#"data: {"responseId":"r1","candidates":[{"content":{"parts":[{"text":"hi"}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        ],
        Some("given-model"),
    );
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let outcome = logger(Arc::clone(&cost))
        .handle(&fixture.call())
        .expect("the chunk builds a response");
    let result = result_json(&outcome);
    assert_eq!(field(&result, "model"), &Json::Null);
    assert_eq!(as_text(kwarg(&outcome, "model")), "given-model");
    assert_eq!(logged_model(&fixture.logging_obj), "given-model");
    assert_eq!(cost.calls()[0].model, "given-model");
    assert_eq!(cost.calls()[0].response_model, None);
}

#[test]
fn chunks_empty_model_falls_back_to_the_url_model() {
    let fixture = collected(
        STREAM_ROUTE
            .replace("gemini-2.0-flash", "url-model")
            .as_str(),
        &[
            r#"data: {"responseId":"r1","candidates":[{"content":{"parts":[{"text":"hi"}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        ],
        Some(""),
    );
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let outcome = logger(Arc::clone(&cost))
        .handle(&fixture.call())
        .expect("the chunk builds a response");
    assert_eq!(field(&result_json(&outcome), "model"), &Json::Null);
    assert_eq!(as_text(kwarg(&outcome, "model")), "url-model");
    assert_eq!(logged_model(&fixture.logging_obj), "url-model");
}

#[test]
fn chunks_no_lines_returns_no_result_with_the_error_text() {
    let fixture = collected(STREAM_ROUTE, &[], None);
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let outcome = logger(Arc::clone(&cost))
        .handle(&fixture.call())
        .expect("an empty stream is not an error");
    assert_eq!(outcome.result, None);
    assert_eq!(outcome.kwargs, PassThroughLoggingKwargs::default());
    assert!(logs.contents().contains(MISSING_RESPONSE));
    assert!(cost.calls().is_empty());
}

#[test]
fn chunks_non_generate_route_returns_no_result_with_the_error_text() {
    let fixture = collected(VIDEO_ROUTE, &[r#"{"candidates":[]}"#], None);
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let outcome = logger(Arc::clone(&cost))
        .handle(&fixture.call())
        .expect("a non-generate route is not an error");
    assert_eq!(outcome.result, None);
    assert_eq!(outcome.kwargs, PassThroughLoggingKwargs::default());
    assert!(logs.contents().contains(MISSING_RESPONSE));
    assert!(cost.calls().is_empty());
}

#[test]
fn chunks_push_error_is_returned() {
    let fixture = collected(STREAM_ROUTE, &["[1,2]"], None);
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let error = logger(Arc::clone(&cost))
        .handle(&fixture.call())
        .expect_err("a non-object chunk stops the stream");
    assert!(matches!(error, HandlerError::Leaf { .. }));
    assert!(cost.calls().is_empty());
}

#[test]
fn chunks_generate_content_route_uses_the_decoder() {
    let fixture = collected(
        GENERATE_ROUTE,
        &[
            r#"{"responseId":"r9","modelVersion":"gemini-2.0-flash","candidates":[{"content":{"parts":[{"text":"hi"}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2}}"#,
        ],
        None,
    );
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let outcome = logger(Arc::clone(&cost))
        .handle(&fixture.call())
        .expect("the bare generateContent route builds a response");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "model"), "gemini-2.0-flash");
    assert_eq!(integer(field(&result, "usage"), "total_tokens"), 2);
}

#[test]
fn chunks_stream_without_a_model_logs_no_model() {
    let fixture = collected(GENERATE_ROUTE, &[NO_MODEL_BODY], Some("given-model"));
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let outcome = logger(Arc::clone(&cost))
        .handle(&fixture.call())
        .expect("the chunk builds a response");
    let result = result_json(&outcome);
    assert_eq!(field(&result, "model"), &Json::Null);
    assert_eq!(as_text(kwarg(&outcome, "model")), "given-model");
    assert_eq!(logged_model(&fixture.logging_obj), "given-model");
}

#[test]
fn route_interactions_uses_the_request_model_when_the_reply_names_none() {
    let mut fixture = fixture(
        INTERACTIONS_ROUTE,
        r#"{"usage":{"total_input_tokens":3,"total_output_tokens":2}}"#,
    );
    fixture.request_body.insert(
        "model".to_owned(),
        serde_json::Value::String("request-model".to_owned()),
    );
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let outcome = logger(Arc::clone(&cost))
        .gemini_passthrough_handler(gemini_input(&fixture))
        .expect("the interactions reply is logged");
    assert_eq!(text(&result_json(&outcome), "model"), "request-model");
    assert_eq!(cost.calls()[0].model, "request-model");
}

#[test]
fn route_predict_long_running_keeps_the_request_resolution() {
    let mut fixture = fixture(VIDEO_ROUTE, VIDEO_BODY);
    fixture.request_body.insert(
        "parameters".to_owned(),
        serde_json::json!({"resolution": " "}),
    );
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let outcome = logger(Arc::clone(&cost))
        .gemini_passthrough_handler(gemini_input(&fixture))
        .expect("the video operation is logged");
    let result = result_json(&outcome);
    let usage = field(&result, "usage");
    assert_eq!(field(usage, "video_resolution"), &Json::Text(String::new()));
}

#[test]
fn payload_usage_keeps_the_prompt_modality_details() {
    let fixture = fixture(GENERATE_ROUTE, DETAILS_BODY);
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let outcome = logger(Arc::clone(&cost))
        .gemini_passthrough_handler(gemini_input(&fixture))
        .expect("the detailed usage is logged");
    let result = result_json(&outcome);
    let usage = field(&result, "usage");
    assert_eq!(
        field(usage, "prompt_tokens_details"),
        &json(r#"{"text_tokens":3}"#)
    );
}

#[test]
fn relay_projection_carries_the_logged_model_and_details() {
    let logging_obj = support::logging_object();
    logging_obj.edit(&mut |state| {
        state.model = "state-model".to_owned();
        state.custom_llm_provider = Some("gemini".to_owned());
        state
            .model_call_details
            .insert("model".to_owned(), Json::Text("detail-model".to_owned()));
        state.model_call_details.insert(
            "custom_llm_provider".to_owned(),
            Json::Text("vertex_ai".to_owned()),
        );
    });
    let relay = RelayLogging::project(&logging_obj);
    assert_eq!(relay.logging.model.as_deref(), Some("state-model"));
    assert_eq!(
        relay.logging.custom_llm_provider,
        Some(CustomLlmProvider::Gemini)
    );
    assert_eq!(
        relay.logging.model_call_details.model.as_deref(),
        Some("detail-model")
    );
    assert_eq!(
        relay.logging.model_call_details.custom_llm_provider,
        Some(CustomLlmProvider::VertexAi)
    );
}

#[test]
fn relay_projection_drops_an_empty_model_and_unknown_provider() {
    let logging_obj = support::logging_object();
    logging_obj.edit(&mut |state| {
        state
            .model_call_details
            .insert("model".to_owned(), Json::Text(String::new()));
        state.model_call_details.insert(
            "custom_llm_provider".to_owned(),
            Json::Text("openai".to_owned()),
        );
    });
    let relay = RelayLogging::project(&logging_obj);
    assert_eq!(relay.logging.model, None);
    assert_eq!(relay.logging.custom_llm_provider, None);
    assert_eq!(relay.logging.model_call_details.model, None);
    assert_eq!(relay.logging.model_call_details.custom_llm_provider, None);
}

#[test]
fn relay_kwargs_write_the_cost_model_and_provider() {
    let requested = kwargs(&[("request_id", Json::Text("r1".to_owned()))]);
    let built = relay_kwargs(
        &requested,
        ResponseLoggingKwargs {
            response_cost: Some(Decimal::new(125, 2)),
            model: Some("gemini-2.0-flash".to_owned()),
            custom_llm_provider: Some(CustomLlmProvider::Gemini),
        },
    );
    assert_eq!(
        built.entries.get("request_id"),
        Some(&Json::Text("r1".to_owned()))
    );
    assert_eq!(
        number_text(built.entries.get("response_cost").expect("a cost")),
        "1.25"
    );
    assert_eq!(
        as_text(built.entries.get("model").expect("a model")),
        "gemini-2.0-flash"
    );
    assert_eq!(
        as_text(
            built
                .entries
                .get("custom_llm_provider")
                .expect("a provider")
        ),
        "gemini"
    );
}

#[test]
fn chunks_without_usage_count_the_assembled_text() {
    let fixture = collected(
        STREAM_ROUTE,
        &[
            r#"data: {"responseId":"r1","modelVersion":"gemini-2.0-flash","candidates":[{"content":{"parts":[{"text":"Hello world"}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        ],
        None,
    );
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let outcome = logger(Arc::clone(&cost))
        .handle(&fixture.call())
        .expect("the chunk builds a response");
    let result = result_json(&outcome);
    let usage = field(&result, "usage");
    assert_eq!(integer(usage, "prompt_tokens"), 0);
    assert_eq!(integer(usage, "completion_tokens"), 2);
    assert_eq!(integer(usage, "total_tokens"), 2);
    assert_eq!(cost.calls()[0].usage, Some((0, 2, 2)));
}

#[test]
fn assembly_silent_hooks_accept_every_write() {
    let payload = Payload::Bool(false);
    assert!(assembly::SilentLogger.debug(&payload));
    assert!(assembly::SilentLogger.info(&payload));
    assert!(assembly::SilentLogger.error(&payload));
    assert!(!assembly::SilentLogger.debug_enabled());
    assert!(assembly::SilentConsole.print(&payload));
}

#[test]
fn assembly_no_costs_prices_nothing() {
    let hidden = JsonObject::new();
    let usage = WireUsage::default();
    let input = CostInput {
        model: "gemini-2.0-flash",
        usage: &usage,
        hidden_params: &hidden,
    };
    assert_eq!(assembly::NoCosts.completion_cost(&input, None), Ok(None));
}

#[test]
fn reply_renders_numeric_extras_in_their_json_type() {
    assert_eq!(reply::extra_json(&UsageNumber::Integer(7)), json("7"));
    assert_eq!(reply::extra_json(&UsageNumber::Boolean(true)), json("true"));
}

#[test]
fn reply_drops_the_unset_message_fields() {
    let fixture = fixture(GENERATE_ROUTE, GENERATE_BODY);
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let outcome = logger(Arc::clone(&cost))
        .gemini_passthrough_handler(gemini_input(&fixture))
        .expect("the generateContent reply is logged");
    let result = result_json(&outcome);
    let Json::Object(message) = field(choices(&result), "message") else {
        panic!("the message is an object");
    };
    assert!(
        message.get("reasoning_content").is_none(),
        "reasoning_content stays out of the message"
    );
    assert!(
        message.get("audio").is_none(),
        "audio stays out of the message"
    );
    assert!(
        message.get("annotations").is_none(),
        "annotations stays out of the message"
    );
    assert!(
        message.get("provider_specific_fields").is_none(),
        "provider_specific_fields stays out of the message"
    );
}

fn choices(result: &Json) -> &Json {
    let Json::List(rows) = field(result, "choices") else {
        panic!("the choices are a list");
    };
    rows.first().expect("one choice")
}
