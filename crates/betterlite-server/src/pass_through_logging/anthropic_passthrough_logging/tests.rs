#![cfg(test)]

mod support;

use std::str::from_utf8;
use std::sync::Arc;

use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, CollectedChunksHandler as _, EndpointType, HandlerError, HandlerOutcome,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use self::support::{
    BATCH, BATCH_RESPONSE_ID, CALL_ID, Exchange, PricedCall, RESULTS, RecordingCost,
    RecordingRegistry, STREAM, at, call_log, capture_logs, kwargs, logger, logging_object,
    model_response, object, start_time, text, text_at,
};
use super::AnthropicPassthroughError;
use crate::pass_through_logging::interrupted_stream_usage::InterruptedStreamUsageError;
use crate::pass_through_logging::stream_fields::{Chunk, StreamFieldError};

const MESSAGES_ROUTE: &str = "/anthropic/v1/messages";

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn priced() -> Json {
    decimal_json(Decimal::new(125, 2))
}

#[tokio::test]
async fn anthropic_batch_create_on_the_collection_route_logs_the_batch() {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    let mut exchange = Exchange::message("/anthropic/v1/messages/batches");
    exchange.response_text = BATCH;
    exchange.request_body = object(&[("model", text("claude-sonnet-4-5"))]);
    let metadata = object(&[("user_api_key_user_id", text("user-7"))]);
    exchange.kwargs = kwargs(&[
        ("call_type", text("pass_through_endpoint")),
        (
            "litellm_params",
            Json::Object(object(&[("metadata", Json::Object(metadata))])),
        ),
    ]);
    let outcome = exchange
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    let result = model_response(&outcome);
    assert_eq!(text_at(result, &["id"]).as_deref(), Some(BATCH_RESPONSE_ID));
    assert_eq!(text_at(result, &["object"]).as_deref(), Some("batch"));
    assert_eq!(
        text_at(result, &["model"]).as_deref(),
        Some("claude-sonnet-4-5")
    );
    let entries = &outcome.kwargs.entries;
    assert_eq!(
        entries.get("call_type"),
        Some(&text("pass_through_endpoint"))
    );
    assert_eq!(
        entries.get("response_cost"),
        Some(&decimal_json(Decimal::ZERO))
    );
    assert_eq!(entries.get("model"), Some(&text("claude-sonnet-4-5")));
    assert_eq!(entries.get("batch_id"), Some(&text("msgbatch_1")));
    assert!(matches!(entries.get("unified_object_id"), Some(Json::Text(id)) if !id.is_empty()));
    assert_eq!(entries.get("batch_job_state"), Some(&text("in_progress")));
    assert!(cost.calls().is_empty(), "a batch record is never priced");
    let registrations = registry.registrations();
    assert_eq!(registrations.len(), 1);
    assert_eq!(
        registrations[0].registrant.user_id.as_deref(),
        Some("user-7")
    );
}

#[test]
fn anthropic_batch_results_download_takes_the_batch_branch() {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    let mut exchange = Exchange::message("/anthropic/v1/messages/batches/msgbatch_1/results");
    exchange.response_text = RESULTS;
    let outcome = exchange
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    let result = model_response(&outcome);
    assert_eq!(
        text_at(result, &["model"]).as_deref(),
        Some("anthropic_batch")
    );
    let entries = &outcome.kwargs.entries;
    assert_eq!(entries.get("batch_job_state"), Some(&text("failed")));
    assert_eq!(entries.get("model"), Some(&text("anthropic_batch")));
    assert_eq!(entries.get("batch_id"), None);
    assert_eq!(entries.get("unified_object_id"), None);
    assert!(cost.calls().is_empty());
}

#[rstest]
#[case::client_error(400)]
#[case::created(201)]
fn anthropic_batch_route_without_200_takes_the_chat_transform(#[case] status_code: u16) {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    let mut exchange = Exchange::message("/anthropic/v1/messages/batches");
    exchange.status_code = status_code;
    let outcome = exchange
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    let result = model_response(&outcome);
    assert_eq!(
        text_at(result, &["object"]).as_deref(),
        Some("chat.completion")
    );
    assert_eq!(outcome.kwargs.entries.get("response_cost"), Some(&priced()));
    assert_eq!(cost.calls().len(), 1);
    assert!(registry.registrations().is_empty());
}

#[rstest]
#[case::litellm_params_missing(None, "default-user")]
#[case::litellm_params_null(Some(Json::Null), "default-user")]
#[case::metadata_null(Some(Json::Object(object(&[("metadata", Json::Null)]))), "default-user")]
#[case::metadata_given(
    Some(Json::Object(object(&[("metadata", Json::Object(object(&[("user_api_key_user_id", text("user-9"))])))]))),
    "user-9"
)]
#[tokio::test]
async fn anthropic_batch_request_metadata_comes_from_litellm_params(
    #[case] litellm_params: Option<Json>,
    #[case] user_id: &str,
) {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    let mut exchange = Exchange::message("/anthropic/v1/messages/batches");
    exchange.response_text = BATCH;
    if let Some(params) = litellm_params {
        exchange.kwargs = kwargs(&[("litellm_params", params)]);
    }
    exchange
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    let registrations = registry.registrations();
    assert_eq!(registrations.len(), 1);
    assert_eq!(
        registrations[0].registrant.user_id.as_deref(),
        Some(user_id)
    );
}

#[rstest]
#[case::results_after_batches("/anthropic/messages/batches/msgbatch_1/results", 200, true)]
#[case::results_before_batches("/results/anthropic/messages/batches", 200, true)]
#[case::v1_results_without_200("/anthropic/v1/messages/batches/msgbatch_1/results", 201, true)]
#[case::batches_without_results("/anthropic/messages/batches/msgbatch_1", 200, false)]
#[case::results_without_batches("/anthropic/v1/messages/results", 200, false)]
#[case::other_case("/anthropic/Messages/Batches/msgbatch_1/Results", 200, false)]
fn anthropic_config_needs_both_route_texts_for_the_results_transform(
    #[case] url_route: &str,
    #[case] status_code: u16,
    #[case] batches_results: bool,
) {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    let mut exchange = Exchange::message(url_route);
    exchange.status_code = status_code;
    exchange.response_text = RESULTS;
    let handled = exchange.handle(&logger(&cost, &registry), &mut log);
    if batches_results {
        let outcome = handled.unwrap();
        let result = model_response(&outcome);
        assert_eq!(at(result, &["usage", "prompt_tokens"]), Some(&number(13)));
        assert_eq!(
            at(result, &["usage", "completion_tokens"]),
            Some(&number(7))
        );
    } else {
        assert!(matches!(
            handled,
            Err(AnthropicPassthroughError::ChatTransform { .. })
        ));
    }
}

#[rstest]
#[case::fast(Some(text("fast")), Some("fast"))]
#[case::empty_text(Some(text("")), Some(""))]
#[case::not_text(Some(number(5)), None)]
#[case::absent(None, None)]
fn anthropic_chat_transform_gets_the_request_speed(
    #[case] speed: Option<Json>,
    #[case] priced_speed: Option<&str>,
) {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    let mut exchange = Exchange::message(MESSAGES_ROUTE);
    if let Some(value) = speed {
        exchange.request_body = object(&[("speed", value)]);
    }
    let outcome = exchange
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    assert_eq!(
        text_at(model_response(&outcome), &["usage", "speed"]).as_deref(),
        priced_speed
    );
    assert_eq!(
        cost.calls(),
        vec![PricedCall {
            model: "claude-sonnet-4-5".to_owned(),
            provider: None,
            speed: priced_speed.map(str::to_owned),
        }]
    );
}

#[test]
fn anthropic_payload_cost_failure_keeps_the_steps_before_it() {
    let (logs, _guard) = capture_logs();
    let cost = RecordingCost::failing();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[("stream", Json::Bool(true))]);
    let mut exchange = Exchange::message(MESSAGES_ROUTE);
    exchange.kwargs = kwargs(&[("call_type", text("pass_through_endpoint"))]);
    let outcome = exchange
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    assert_eq!(
        outcome.kwargs,
        kwargs(&[("call_type", text("pass_through_endpoint"))])
    );
    let result = model_response(&outcome);
    let id = text_at(result, &["id"]).unwrap();
    assert!(
        id.starts_with("chatcmpl-"),
        "the id was never stamped: {id}"
    );
    let details = &log.model_call_details;
    assert_eq!(details.get("response_cost"), None);
    assert_eq!(details.get("model"), Some(&text("")));
    assert_eq!(details.get("custom_llm_provider"), None);
    let stored = details.get("complete_streaming_response").unwrap();
    assert_eq!(text_at(stored, &["id"]), Some(id));
    assert!(logs.contents().contains(
        "Error creating Anthropic response logging payload: the response cost could not be computed"
    ));
}

#[test]
fn anthropic_payload_prices_the_resolved_costing_model() {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let params = object(&[("model", text("claude-deploy"))]);
    let mut log = call_log(&[
        ("litellm_params", Json::Object(params)),
        ("custom_llm_provider", text("anthropic")),
    ]);
    let mut exchange = Exchange::message(MESSAGES_ROUTE);
    exchange.response_text = r#"{"id":"msg_up","type":"message","role":"assistant","model":"","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":5}}"#;
    let outcome = exchange
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    assert_eq!(
        cost.calls(),
        vec![PricedCall {
            model: "anthropic/claude-deploy".to_owned(),
            provider: Some("anthropic".to_owned()),
            speed: None,
        }]
    );
    assert_eq!(
        outcome.kwargs.entries.get("model"),
        Some(&text("claude-deploy"))
    );
    assert_eq!(outcome.kwargs.entries.get("response_cost"), Some(&priced()));
    assert_eq!(
        text_at(model_response(&outcome), &["model"]).as_deref(),
        Some("claude-deploy")
    );
    let details = &log.model_call_details;
    assert_eq!(details.get("model"), Some(&text("claude-deploy")));
    assert_eq!(details.get("response_cost"), Some(&priced()));
}

#[test]
fn anthropic_payload_cache_hit_costs_zero() {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[("cache_hit", Json::Bool(true))]);
    let outcome = Exchange::message(MESSAGES_ROUTE)
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    let zero = decimal_json(Decimal::ZERO);
    assert_eq!(outcome.kwargs.entries.get("response_cost"), Some(&zero));
    assert_eq!(log.model_call_details.get("response_cost"), Some(&zero));
    assert!(cost.calls().is_empty());
}

#[rstest]
#[case::stream_true(Some(Json::Bool(true)), true)]
#[case::stream_false(Some(Json::Bool(false)), false)]
#[case::stream_null(Some(Json::Null), false)]
#[case::stream_text(Some(text("true")), false)]
#[case::stream_absent(None, false)]
fn anthropic_payload_stores_the_complete_response_only_for_a_stream(
    #[case] stream: Option<Json>,
    #[case] stored: bool,
) {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    if let Some(flag) = stream {
        log.model_call_details.insert("stream".to_owned(), flag);
    }
    let outcome = Exchange::message(MESSAGES_ROUTE)
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    let complete = log.model_call_details.get("complete_streaming_response");
    if stored {
        let copy = complete.unwrap();
        assert_eq!(copy, model_response(&outcome));
        assert_eq!(text_at(copy, &["id"]).as_deref(), Some("msg_up"));
        assert_eq!(
            text_at(copy, &["model"]).as_deref(),
            Some("claude-sonnet-4-5")
        );
    } else {
        assert_eq!(complete, None);
    }
}

#[test]
fn anthropic_payload_end_user_replaces_proxy_server_request() {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    let earlier = object(&[
        ("url", text("https://proxy/v1/messages")),
        ("headers", Json::Object(object(&[]))),
    ]);
    let metadata = object(&[("user_api_key_team_id", text("team-1"))]);
    let request_body = object(&[("user", text("end-user-1"))]);
    let mut exchange = Exchange::message(MESSAGES_ROUTE);
    exchange.kwargs = kwargs(&[
        (
            "litellm_params",
            Json::Object(object(&[
                ("metadata", Json::Object(metadata.clone())),
                ("proxy_server_request", Json::Object(earlier)),
            ])),
        ),
        (
            "passthrough_logging_payload",
            Json::Object(object(&[("request_body", Json::Object(request_body))])),
        ),
    ]);
    let outcome = exchange
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    let replaced = object(&[(
        "body",
        Json::Object(object(&[("user", text("end-user-1"))])),
    )]);
    assert_eq!(
        outcome.kwargs.entries.get("litellm_params"),
        Some(&Json::Object(object(&[
            ("metadata", Json::Object(metadata)),
            ("proxy_server_request", Json::Object(replaced)),
        ])))
    );
}

#[rstest]
#[case::litellm_params_missing(None)]
#[case::litellm_params_null(Some(Json::Null))]
fn anthropic_payload_end_user_creates_litellm_params(#[case] litellm_params: Option<Json>) {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    let payload = object(&[(
        "request_body",
        Json::Object(object(&[("user", text("end-user-2"))])),
    )]);
    let mut entries = vec![("passthrough_logging_payload", Json::Object(payload))];
    if let Some(params) = litellm_params {
        entries.push(("litellm_params", params));
    }
    let mut exchange = Exchange::message(MESSAGES_ROUTE);
    exchange.kwargs = kwargs(&entries);
    let outcome = exchange
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    let request = object(&[(
        "body",
        Json::Object(object(&[("user", text("end-user-2"))])),
    )]);
    assert_eq!(
        outcome.kwargs.entries.get("litellm_params"),
        Some(&Json::Object(object(&[(
            "proxy_server_request",
            Json::Object(request)
        )])))
    );
    assert_eq!(
        text_at(model_response(&outcome), &["id"]).as_deref(),
        Some("msg_up")
    );
}

#[rstest]
#[case::payload_missing(None)]
#[case::payload_empty(Some(object(&[])))]
#[case::request_body_empty(Some(object(&[("request_body", Json::Object(object(&[])))])))]
#[case::request_body_not_object(Some(object(&[("request_body", text("user=end-user-3"))])))]
#[case::no_user(Some(object(&[("request_body", Json::Object(object(&[("model", text("claude-sonnet-4-5"))])))])))]
fn anthropic_payload_without_end_user_leaves_litellm_params(#[case] payload: Option<JsonObject>) {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    let params = Json::Object(object(&[("metadata", Json::Object(object(&[])))]));
    let mut entries = vec![("litellm_params", params.clone())];
    if let Some(fields) = payload {
        entries.push(("passthrough_logging_payload", Json::Object(fields)));
    }
    let mut exchange = Exchange::message(MESSAGES_ROUTE);
    exchange.kwargs = kwargs(&entries);
    let outcome = exchange
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    assert_eq!(outcome.kwargs.entries.get("litellm_params"), Some(&params));
}

#[test]
fn anthropic_payload_litellm_params_of_the_wrong_type_stops_the_payload() {
    let (logs, _guard) = capture_logs();
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    let payload = object(&[(
        "request_body",
        Json::Object(object(&[("user", text("end-user-4"))])),
    )]);
    let mut exchange = Exchange::message(MESSAGES_ROUTE);
    exchange.kwargs = kwargs(&[
        ("litellm_params", text("not an object")),
        ("passthrough_logging_payload", Json::Object(payload)),
    ]);
    let outcome = exchange
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    assert_eq!(outcome.kwargs.entries.get("response_cost"), Some(&priced()));
    assert_eq!(
        outcome.kwargs.entries.get("litellm_params"),
        Some(&text("not an object"))
    );
    let id = text_at(model_response(&outcome), &["id"]).unwrap();
    assert!(
        id.starts_with("chatcmpl-"),
        "the id was never stamped: {id}"
    );
    assert_eq!(log.model_call_details.get("custom_llm_provider"), None);
    assert!(logs.contents().contains(
        "Error creating Anthropic response logging payload: kwargs litellm_params is not a JSON object"
    ));
}

#[rstest]
#[case::upstream_id(r#""msg_up""#, "msg_up")]
#[case::empty_id(r#""""#, CALL_ID)]
#[case::null_id("null", CALL_ID)]
#[case::number_id("5", CALL_ID)]
fn anthropic_payload_response_id_falls_back_to_the_call_id(
    #[case] id: &str,
    #[case] expected: &str,
) {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    let body = format!(
        r#"{{"id":{id},"type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{{"type":"text","text":"hello"}}],"stop_reason":"end_turn","stop_sequence":null,"usage":{{"input_tokens":10,"output_tokens":5}}}}"#
    );
    let mut exchange = Exchange::message(MESSAGES_ROUTE);
    exchange.response_text = &body;
    let outcome = exchange
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    assert_eq!(
        text_at(model_response(&outcome), &["id"]).as_deref(),
        Some(expected)
    );
}

#[rstest]
#[case::absent(None, "anthropic")]
#[case::null(Some(Json::Null), "anthropic")]
#[case::empty(Some(text("")), "anthropic")]
#[case::already_set(Some(text("bedrock")), "bedrock")]
fn anthropic_payload_sets_the_provider_only_when_unset(
    #[case] provider: Option<Json>,
    #[case] expected: &str,
) {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    if let Some(value) = provider {
        log.model_call_details
            .insert("custom_llm_provider".to_owned(), value);
    }
    Exchange::message(MESSAGES_ROUTE)
        .handle(&logger(&cost, &registry), &mut log)
        .unwrap();
    assert_eq!(
        log.model_call_details.get("custom_llm_provider"),
        Some(&text(expected))
    );
    assert_eq!(
        log.model_call_details.get("model"),
        Some(&text("claude-sonnet-4-5"))
    );
}

fn stream_lines() -> Vec<String> {
    STREAM.iter().map(|line| (*line).to_owned()).collect()
}

#[test]
fn anthropic_chunks_complete_stream_is_logged() {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let logging_obj = logging_object(Some(call_log(&[("stream", Json::Bool(true))])));
    let passthrough = PassThroughEndpointLogging {
        injected_log_dispatch: None,
    };
    let request_body = Json::Object(object(&[]));
    let lines = stream_lines();
    let call = CollectedChunksCall {
        logging_obj: &logging_obj,
        passthrough: &passthrough,
        url_route: MESSAGES_ROUTE,
        request_body: Some(&request_body),
        endpoint_type: EndpointType::Anthropic,
        start_time: start_time().fixed_offset(),
        end_time: start_time().fixed_offset(),
        lines: &lines,
        model: Some("ignored-model"),
    };
    let outcome = logger(&cost, &registry).handle(&call).unwrap();
    let result = model_response(&outcome);
    assert_eq!(text_at(result, &["id"]).as_deref(), Some("msg_1"));
    assert_eq!(text_at(result, &["model"]).as_deref(), Some("claude-test"));
    assert_eq!(
        outcome.kwargs.entries.get("model"),
        Some(&text("claude-test"))
    );
    assert_eq!(outcome.kwargs.entries.get("response_cost"), Some(&priced()));
    let calls = cost.calls();
    assert!(
        calls.len() >= 2,
        "the stream build prices through the lent cost service before the payload does: {calls:?}"
    );
    assert_eq!(
        calls.last().map(|priced_call| priced_call.model.as_str()),
        Some("claude-test")
    );
    let stored = logging_obj
        .read(&mut |state| {
            state
                .model_call_details
                .get("complete_streaming_response")
                .cloned()
        })
        .flatten()
        .unwrap();
    assert_eq!(&stored, result);
}

#[test]
fn anthropic_chunks_unbuildable_stream_logs_and_returns_nothing() {
    let (logs, _guard) = capture_logs();
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    let request_body = object(&[("model", text("claude-test"))]);
    let outcome = logger(&cost, &registry)
        .log_collected_chunks(&mut log, &request_body, &[])
        .unwrap();
    assert_eq!(outcome, HandlerOutcome::default());
    assert_eq!(outcome.kwargs, PassThroughLoggingKwargs::default());
    assert!(logs.contents().contains(
        "Unable to build complete streaming response for Anthropic passthrough endpoint, not logging..."
    ));
    assert!(cost.calls().is_empty());
}

#[test]
fn anthropic_chunks_invalid_utf8_is_a_typed_error() {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let mut log = call_log(&[]);
    let request_body = object(&[]);
    let chunks = [Chunk::Bytes(vec![0xff, 0xfe])];
    let handled = logger(&cost, &registry).log_collected_chunks(&mut log, &request_body, &chunks);
    let Err(error) = handled else {
        panic!("invalid UTF-8 must fail the build");
    };
    assert!(matches!(
        error,
        AnthropicPassthroughError::MessageStart { .. }
    ));
    assert!(matches!(
        HandlerError::from(error),
        HandlerError::Leaf { .. }
    ));
}

#[test]
fn anthropic_chunks_without_a_call_log_is_an_error() {
    let cost = RecordingCost::priced();
    let registry = Arc::new(RecordingRegistry::default());
    let logging_obj = logging_object(None);
    let passthrough = PassThroughEndpointLogging {
        injected_log_dispatch: None,
    };
    let lines = stream_lines();
    let call = CollectedChunksCall {
        logging_obj: &logging_obj,
        passthrough: &passthrough,
        url_route: MESSAGES_ROUTE,
        request_body: None,
        endpoint_type: EndpointType::Anthropic,
        start_time: start_time().fixed_offset(),
        end_time: start_time().fixed_offset(),
        lines: &lines,
        model: None,
    };
    let Err(HandlerError::Leaf { source }) = logger(&cost, &registry).handle(&call) else {
        panic!("a held call log must fail the handler");
    };
    assert_eq!(
        source.to_string(),
        "the call log is held by the success dispatch"
    );
}

#[test]
fn anthropic_stream_build_error_converts_into_a_handler_error() {
    let bytes = vec![0xff_u8];
    let utf8 = from_utf8(&bytes).unwrap_err();
    let built = InterruptedStreamUsageError::Utf8(StreamFieldError::Utf8 {
        index: 0,
        source: utf8,
    });
    let error = AnthropicPassthroughError::from(built);
    assert!(matches!(
        error,
        AnthropicPassthroughError::StreamBuild { .. }
    ));
    let HandlerError::Leaf { source } = HandlerError::from(error) else {
        panic!("a leaf error stays a leaf error");
    };
    assert!(
        source
            .to_string()
            .starts_with("the streamed Anthropic response could not be built")
    );
}
