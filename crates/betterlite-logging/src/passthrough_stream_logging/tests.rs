#![cfg(test)]

mod support;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::current;

use betterlite_models::decisions::Json;
use rstest::rstest;

use self::support::{
    FakeHandler, FakeTinyfish, PASS_THROUGH, PanickingHandler, ProbeStream, RecordingDispatch,
    RecordingLogger, billing_handle, capture_logs, corpus_chunks, dispatch_handle, handler_handle,
    json, object, object_state, probe, providers_with, route_default, route_stream, state, time,
};
use super::build::build_passthrough_logging_result;
use super::lines::convert_raw_bytes_to_str_lines;
use super::model::extract_model_for_cost_injection;
use super::object::SuccessHandlerDispatch;
use super::types::{CollectedChunksHandler, EndpointType, HandlerError, HandlerOutcome};
use crate::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use crate::passthrough_logging_state::response::{PassThroughResponse, PassThroughResponseObject};
use crate::registry::CallbackRegistry;

const VERTEX_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/l/publishers/google/models/gemini-1.5-pro:streamGenerateContent";

fn fallback_text(lines: &[String]) -> String {
    format!(
        "cannot parse chunks to standard response object. Chunks={}",
        serde_json::to_string(lines).unwrap()
    )
}

fn response() -> PassThroughResponse {
    PassThroughResponse::ModelResponse(json(r#"{"id":"resp-1"}"#))
}

fn outcome_with_kwargs() -> HandlerOutcome {
    let mut kwargs = PassThroughLoggingKwargs::default();
    kwargs
        .entries
        .insert("model".to_owned(), Json::Text("gpt-4o".to_owned()));
    HandlerOutcome {
        result: Some(response()),
        kwargs,
    }
}

#[test]
fn lines_join_before_decode_keeps_a_split_multibyte_character() {
    let text = "data: h\u{e9}llo \u{1f600}";
    let emoji_start = text.len().saturating_sub("\u{1f600}".len());
    let bytes = text.as_bytes();
    let chunks = vec![
        bytes[..emoji_start].to_vec(),
        bytes[emoji_start..=emoji_start].to_vec(),
        bytes[emoji_start.saturating_add(1)..].to_vec(),
    ];

    assert_eq!(convert_raw_bytes_to_str_lines(&chunks), vec![text]);
}

#[test]
fn lines_cut_inside_a_character_keep_the_earlier_lines() {
    let chunks = vec![b"data: 1\n".to_vec(), vec![0xF0, 0x9F]];

    assert_eq!(
        convert_raw_bytes_to_str_lines(&chunks),
        vec!["data: 1".to_owned(), "\u{FFFD}".to_owned()]
    );
}

#[test]
fn lines_replace_invalid_utf8_with_the_replacement_character() {
    let chunks = vec![vec![b'd', b'a', b't', b'a', b':', b' ', 0xFF, b'x']];

    assert_eq!(
        convert_raw_bytes_to_str_lines(&chunks),
        vec!["data: \u{FFFD}x".to_owned()]
    );
}

#[test]
fn lines_trim_every_line_and_drop_the_blank_ones() {
    let chunks = vec![b"  \n\rd a t a\r\n\n \t \ndata: x \r\n\n".to_vec()];

    assert_eq!(
        convert_raw_bytes_to_str_lines(&chunks),
        vec!["d a t a".to_owned(), "data: x".to_owned()]
    );
}

#[test]
fn lines_split_on_newline_only() {
    let chunks = vec![b"a\x0bb\x0cc".to_vec()];

    assert_eq!(
        convert_raw_bytes_to_str_lines(&chunks),
        vec!["a\u{b}b\u{c}c".to_owned()]
    );
}

#[test]
fn lines_strip_python_whitespace_separators() {
    let chunks = vec![b"data: {}\x1e\n\x1f\n\x1cdata\x1d\n".to_vec()];

    assert_eq!(
        convert_raw_bytes_to_str_lines(&chunks),
        vec!["data: {}".to_owned(), "data".to_owned()]
    );
}

#[rstest]
fn replays_the_passthrough_stream_lines_corpus(
    #[files("fuzz/corpus/passthrough_stream_lines/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    let chunked = convert_raw_bytes_to_str_lines(&corpus_chunks(&bytes));
    let joined = match bytes.split_first() {
        Some((_, content)) => convert_raw_bytes_to_str_lines(&[content.to_vec()]),
        None => Vec::new(),
    };
    assert_eq!(chunked, joined);
}

#[test]
fn model_body_wins_over_the_logging_object_and_the_url() {
    let body = json(r#"{"model":"body-model"}"#);

    let model = extract_model_for_cost_injection(
        Some(&body),
        Some("log-model"),
        VERTEX_ROUTE,
        EndpointType::VertexAi,
    );

    assert_eq!(model.as_deref(), Some("body-model"));
}

#[test]
fn model_body_empty_string_falls_through_to_the_logging_object() {
    let body = json(r#"{"model":""}"#);

    let model = extract_model_for_cost_injection(
        Some(&body),
        Some("log-model"),
        "/v1/messages",
        EndpointType::Anthropic,
    );

    assert_eq!(model.as_deref(), Some("log-model"));
}

#[test]
fn model_body_non_string_falls_through_to_the_logging_object() {
    let body = json(r#"{"model":3}"#);

    let model = extract_model_for_cost_injection(
        Some(&body),
        Some("log-model"),
        VERTEX_ROUTE,
        EndpointType::VertexAi,
    );

    assert_eq!(model.as_deref(), Some("log-model"));
}

#[test]
fn model_body_that_is_not_an_object_falls_through() {
    let body = json(r#"["model","gpt-4o"]"#);

    let model =
        extract_model_for_cost_injection(Some(&body), None, VERTEX_ROUTE, EndpointType::VertexAi);

    assert_eq!(model.as_deref(), Some("gemini-1.5-pro"));
}

#[test]
fn model_logging_object_used_when_the_body_has_no_model() {
    let body = json(r#"{"messages":[]}"#);

    let model = extract_model_for_cost_injection(
        Some(&body),
        Some("log-model"),
        "/v1/messages",
        EndpointType::Anthropic,
    );

    assert_eq!(model.as_deref(), Some("log-model"));
}

#[test]
fn model_empty_logging_object_value_is_skipped() {
    let model =
        extract_model_for_cost_injection(None, Some(""), VERTEX_ROUTE, EndpointType::VertexAi);

    assert_eq!(model.as_deref(), Some("gemini-1.5-pro"));
}

#[rstest]
#[case::vertex_ai(EndpointType::VertexAi, Some("gemini-1.5-pro"))]
#[case::gemini(EndpointType::Gemini, None)]
#[case::anthropic(EndpointType::Anthropic, None)]
#[case::open_ai(EndpointType::OpenAi, None)]
#[case::tinyfish(EndpointType::Tinyfish, None)]
#[case::generic(EndpointType::Generic, None)]
#[case::decisions(EndpointType::Decisions, None)]
fn model_url_source_applies_to_vertex_ai_only(
    #[case] endpoint_type: EndpointType,
    #[case] expected: Option<&str>,
) {
    let model = extract_model_for_cost_injection(None, None, VERTEX_ROUTE, endpoint_type);

    assert_eq!(model.as_deref(), expected);
}

#[test]
fn model_vertex_url_unknown_is_rejected() {
    let model = extract_model_for_cost_injection(
        None,
        None,
        "https://example.com/v1/generateContent",
        EndpointType::VertexAi,
    );

    assert_eq!(model, None);
}

#[test]
fn model_every_source_empty_is_none() {
    let body = json("{}");

    let model = extract_model_for_cost_injection(
        Some(&body),
        None,
        "/v1/messages",
        EndpointType::Anthropic,
    );

    assert_eq!(model, None);
}

#[rstest]
#[case::anthropic(EndpointType::Anthropic, "anthropic")]
#[case::vertex_ai(EndpointType::VertexAi, "vertex")]
#[case::gemini(EndpointType::Gemini, "gemini")]
#[case::open_ai(EndpointType::OpenAi, "openai")]
fn result_dispatch_calls_only_the_handler_for_the_endpoint_type(
    #[case] endpoint_type: EndpointType,
    #[case] expected: &str,
) {
    let handlers = support::handlers();
    let input = support::input(
        object(
            state(PASS_THROUGH, &[]),
            Arc::new(RecordingDispatch::default()),
        ),
        Arc::clone(&handlers.providers),
        endpoint_type,
    );

    let _built = build_passthrough_logging_result(&input).unwrap();

    let mut called: Vec<&'static str> = Vec::new();
    for handler in [
        &handlers.anthropic,
        &handlers.vertex,
        &handlers.gemini,
        &handlers.openai,
    ] {
        called.extend(handler.calls().into_iter().map(|call| call.handler));
    }
    assert_eq!(called, vec![expected]);
}

#[test]
fn result_dispatch_passes_the_context_and_the_converted_lines() {
    let handlers = support::handlers();
    let logging_obj = object(
        state(PASS_THROUGH, &[]),
        Arc::new(RecordingDispatch::default()),
    );
    let mut input = support::input(
        Arc::clone(&logging_obj),
        Arc::clone(&handlers.providers),
        EndpointType::Anthropic,
    );
    input.url_route = "https://api.anthropic.com/v1/messages".to_owned();
    input.request_body = Some(json(r#"{"model":"claude"}"#));
    input.start_time = time("2024-01-01T00:00:00+00:00");
    input.end_time = time("2024-01-01T00:00:03+00:00");
    input.raw_bytes = vec![b"event: message_stop\n\n".to_vec()];

    let _built = build_passthrough_logging_result(&input).unwrap();

    let calls = handlers.anthropic.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.url_route, "https://api.anthropic.com/v1/messages");
    assert_eq!(call.endpoint_type, EndpointType::Anthropic);
    assert_eq!(call.start_time, input.start_time);
    assert_eq!(call.end_time, input.end_time);
    assert_eq!(call.lines, vec!["event: message_stop".to_owned()]);
    assert_eq!(call.request_body, Some(json(r#"{"model":"claude"}"#)));
}

#[rstest]
#[case::vertex_ai(EndpointType::VertexAi)]
#[case::gemini(EndpointType::Gemini)]
#[case::anthropic(EndpointType::Anthropic)]
#[case::open_ai(EndpointType::OpenAi)]
fn result_dispatch_passes_the_model_to_vertex_and_gemini_only(#[case] endpoint_type: EndpointType) {
    let handlers = support::handlers();
    let mut input = support::input(
        object(
            state(PASS_THROUGH, &[]),
            Arc::new(RecordingDispatch::default()),
        ),
        Arc::clone(&handlers.providers),
        endpoint_type,
    );
    input.model = Some("caller-model".to_owned());

    let _built = build_passthrough_logging_result(&input).unwrap();

    let handler = match endpoint_type {
        EndpointType::VertexAi => &handlers.vertex,
        EndpointType::Gemini => &handlers.gemini,
        EndpointType::Anthropic => &handlers.anthropic,
        EndpointType::OpenAi => &handlers.openai,
        EndpointType::Tinyfish | EndpointType::Generic | EndpointType::Decisions => {
            panic!("this case list names only the four handled endpoint types")
        }
    };
    let calls = handler.calls();
    let expected = match endpoint_type {
        EndpointType::VertexAi | EndpointType::Gemini => Some("caller-model"),
        EndpointType::Anthropic | EndpointType::OpenAi => None,
        EndpointType::Tinyfish | EndpointType::Generic | EndpointType::Decisions => {
            panic!("this case list names only the four handled endpoint types")
        }
    };
    assert_eq!(calls[0].model.as_deref(), expected);
}

#[test]
fn result_dispatch_returns_the_handler_result_and_kwargs() {
    let handler = Arc::new(FakeHandler::returning(
        "anthropic",
        Ok(outcome_with_kwargs()),
    ));
    let shared: Arc<dyn CollectedChunksHandler> = handler_handle(Arc::clone(&handler));
    let input = support::input(
        object(
            state(PASS_THROUGH, &[]),
            Arc::new(RecordingDispatch::default()),
        ),
        providers_with(&shared),
        EndpointType::Anthropic,
    );

    let built = build_passthrough_logging_result(&input).unwrap();

    assert_eq!(built.result, response());
    assert_eq!(
        built.kwargs.entries.get("model"),
        Some(&Json::Text("gpt-4o".to_owned()))
    );
}

#[test]
fn result_dispatch_handler_error_comes_back_as_err() {
    let handler: Arc<dyn CollectedChunksHandler> = Arc::new(FakeHandler::returning(
        "openai",
        Err(HandlerError::leaf(TestLeafError)),
    ));
    let input = support::input(
        object(
            state(PASS_THROUGH, &[]),
            Arc::new(RecordingDispatch::default()),
        ),
        providers_with(&handler),
        EndpointType::OpenAi,
    );

    let built = build_passthrough_logging_result(&input);

    match built {
        Err(HandlerError::Leaf { source }) => {
            assert_eq!(source.to_string(), "the leaf failed");
        }
        other => panic!("expected the leaf error, got {other:?}"),
    }
}

#[rstest]
#[case::tinyfish(EndpointType::Tinyfish)]
#[case::generic(EndpointType::Generic)]
#[case::decisions(EndpointType::Decisions)]
fn result_dispatch_selects_no_handler_for_other_endpoint_types(
    #[case] endpoint_type: EndpointType,
) {
    let handlers = support::handlers();
    let input = support::input(
        object(
            state(PASS_THROUGH, &[]),
            Arc::new(RecordingDispatch::default()),
        ),
        Arc::clone(&handlers.providers),
        endpoint_type,
    );

    let built = build_passthrough_logging_result(&input).unwrap();

    assert_eq!(built.result, expected_fallback(&[]));
    assert_eq!(built.kwargs, PassThroughLoggingKwargs::default());
    for handler in [
        &handlers.anthropic,
        &handlers.vertex,
        &handlers.gemini,
        &handlers.openai,
    ] {
        assert!(handler.calls().is_empty());
    }
}

fn standard_logging_kwargs() -> PassThroughLoggingKwargs {
    let mut kwargs = PassThroughLoggingKwargs::default();
    kwargs.entries.insert(
        STANDARD_LOGGING_OBJECT.to_owned(),
        serde_json::from_str(support::PAYLOAD).unwrap(),
    );
    kwargs
}

fn expected_fallback(lines: &[String]) -> PassThroughResponse {
    PassThroughResponse::Standard(PassThroughResponseObject {
        response: Json::Text(fallback_text(lines)),
    })
}

#[test]
fn result_fallback_for_a_generic_endpoint_lists_the_lines_as_json() {
    let handlers = support::handlers();
    let mut input = support::input(
        object(
            state(PASS_THROUGH, &[]),
            Arc::new(RecordingDispatch::default()),
        ),
        Arc::clone(&handlers.providers),
        EndpointType::Generic,
    );
    input.raw_bytes = vec![b"data: {\"a\":1}\nline two\n".to_vec()];
    let lines = vec!["data: {\"a\":1}".to_owned(), "line two".to_owned()];

    let built = build_passthrough_logging_result(&input).unwrap();

    assert_eq!(built.result, expected_fallback(&lines));
    assert!(fallback_text(&lines).contains("[\"data: {\\\"a\\\":1}\",\"line two\"]"));
}

#[test]
fn result_fallback_for_empty_lines_renders_an_empty_json_array() {
    let handlers = support::handlers();
    let input = support::input(
        object(
            state(PASS_THROUGH, &[]),
            Arc::new(RecordingDispatch::default()),
        ),
        Arc::clone(&handlers.providers),
        EndpointType::Generic,
    );

    let built = build_passthrough_logging_result(&input).unwrap();

    assert_eq!(built.result, expected_fallback(&[]));
    assert!(fallback_text(&[]).ends_with("Chunks=[]"));
}

#[test]
fn result_fallback_keeps_the_kwargs_of_a_handler_without_a_result() {
    let handler: Arc<dyn CollectedChunksHandler> = Arc::new(FakeHandler::returning(
        "openai",
        Ok(HandlerOutcome {
            result: None,
            kwargs: outcome_with_kwargs().kwargs,
        }),
    ));
    let mut input = support::input(
        object(
            state(PASS_THROUGH, &[]),
            Arc::new(RecordingDispatch::default()),
        ),
        providers_with(&handler),
        EndpointType::OpenAi,
    );
    input.raw_bytes = vec![b"event: ping\n\n".to_vec()];
    let lines = vec!["event: ping".to_owned()];

    let built = build_passthrough_logging_result(&input).unwrap();

    assert_eq!(built.result, expected_fallback(&lines));
    assert_eq!(
        built.kwargs.entries.get("model"),
        Some(&Json::Text("gpt-4o".to_owned()))
    );
}

#[test]
fn result_fallback_escapes_json_metacharacters() {
    let handlers = support::handlers();
    let mut input = support::input(
        object(
            state(PASS_THROUGH, &[]),
            Arc::new(RecordingDispatch::default()),
        ),
        Arc::clone(&handlers.providers),
        EndpointType::Generic,
    );
    input.raw_bytes = vec![b"data: \"quoted\"\n\n".to_vec()];
    let lines = vec!["data: \"quoted\"".to_owned()];

    let built = build_passthrough_logging_result(&input).unwrap();

    assert_eq!(built.result, expected_fallback(&lines));
}

#[tokio::test]
async fn evidence_stamps_true_for_a_complete_anthropic_stream() {
    let (logging_obj, fake) =
        support::route_anthropic(b"event: message_stop\ndata: {}\n\n", ProbeStream::Complete).await;

    assert_eq!(
        object_state(&logging_obj).get("prompt_cache_response_complete"),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        fake.stop_inputs.lock().unwrap().as_slice(),
        [b"event: message_stop\ndata: {}\n\n".to_vec()]
    );
}

#[tokio::test]
async fn evidence_stamps_false_without_a_message_stop() {
    let (logging_obj, _fake) =
        support::route_anthropic(b"event: ping\n\n", ProbeStream::NoMessageStop).await;

    assert_eq!(
        object_state(&logging_obj).get("prompt_cache_response_complete"),
        Some(&Json::Bool(false))
    );
}

#[tokio::test]
async fn evidence_stamps_false_for_a_provider_error_frame() {
    let (logging_obj, _fake) = support::route_anthropic(
        b"event: message_stop\nevent: error\n\n",
        ProbeStream::ProviderError,
    )
    .await;

    assert_eq!(
        object_state(&logging_obj).get("prompt_cache_response_complete"),
        Some(&Json::Bool(false))
    );
}

#[tokio::test]
async fn evidence_stamps_false_with_a_non_whitespace_tail() {
    let (logging_obj, _fake) = support::route_anthropic(
        b"event: message_stop\n\ndata: partial",
        ProbeStream::Complete,
    )
    .await;

    assert_eq!(
        object_state(&logging_obj).get("prompt_cache_response_complete"),
        Some(&Json::Bool(false))
    );
}

#[rstest]
#[case::spaces(b"event: message_stop\n\n   ")]
#[case::vertical_tab(b"event: message_stop\ndata: {}\n\n\x0b")]
#[tokio::test]
async fn evidence_stamps_true_with_a_whitespace_only_tail(#[case] raw: &[u8]) {
    let (logging_obj, _fake) = support::route_anthropic(raw, ProbeStream::Complete).await;

    assert_eq!(
        object_state(&logging_obj).get("prompt_cache_response_complete"),
        Some(&Json::Bool(true))
    );
}

#[rstest]
#[case::crlf(b"event: message_stop\r\ndata: {}\r\n\r\n")]
#[case::cr_only(b"event: message_stop\rdata: {}\r\r")]
#[tokio::test]
async fn evidence_stamps_true_for_other_frame_delimiters(#[case] raw: &[u8]) {
    let (logging_obj, _fake) = support::route_anthropic(raw, ProbeStream::Complete).await;

    assert_eq!(
        object_state(&logging_obj).get("prompt_cache_response_complete"),
        Some(&Json::Bool(true))
    );
}

#[tokio::test]
async fn evidence_uses_the_last_frame_boundary_for_the_probe() {
    let (logging_obj, fake) = support::route_anthropic(
        b"event: ping\n\nevent: message_stop\r\n\r\ndata: tail",
        ProbeStream::Complete,
    )
    .await;

    assert_eq!(
        object_state(&logging_obj).get("prompt_cache_response_complete"),
        Some(&Json::Bool(false))
    );
    assert_eq!(
        fake.stop_inputs.lock().unwrap().as_slice(),
        [b"event: ping\n\nevent: message_stop\r\n\r\n".to_vec()]
    );
}

#[tokio::test]
async fn evidence_stamps_false_for_a_non_anthropic_type_and_probes_empty_bytes() {
    let logging_obj = object(
        state(PASS_THROUGH, &[]),
        Arc::new(RecordingDispatch::default()),
    );
    let mut input = support::input(
        Arc::clone(&logging_obj),
        Arc::clone(&support::handlers().providers),
        EndpointType::OpenAi,
    );
    input.raw_bytes = vec![b"event: message_stop\n\n".to_vec()];
    let fake = probe(ProbeStream::Complete);
    let frames = Arc::clone(&fake);

    route_default(input, frames).await;

    assert_eq!(
        object_state(&logging_obj).get("prompt_cache_response_complete"),
        Some(&Json::Bool(false))
    );
    assert_eq!(
        fake.stop_inputs.lock().unwrap().as_slice(),
        [Vec::<u8>::new()]
    );
    assert_eq!(
        fake.error_inputs.lock().unwrap().as_slice(),
        [Vec::<u8>::new()]
    );
}

#[tokio::test]
async fn evidence_stamp_survives_a_failing_handler() {
    let logging_obj = object(
        state(PASS_THROUGH, &[]),
        Arc::new(RecordingDispatch::default()),
    );
    let handler: Arc<dyn CollectedChunksHandler> = Arc::new(FakeHandler::returning(
        "anthropic",
        Err(HandlerError::leaf(TestLeafError)),
    ));
    let mut input = support::input(
        Arc::clone(&logging_obj),
        providers_with(&handler),
        EndpointType::Anthropic,
    );
    input.raw_bytes = vec![b"event: ping\n\n".to_vec()];
    let frames = probe(ProbeStream::NoMessageStop);

    route_default(input, frames).await;

    assert_eq!(
        object_state(&logging_obj).get("prompt_cache_response_complete"),
        Some(&Json::Bool(false))
    );
}

#[tokio::test]
async fn tinyfish_route_returns_without_dispatch_when_a_poller_runs() {
    let dispatch = Arc::new(RecordingDispatch::default());
    let logging_obj = object(
        state(PASS_THROUGH, &[]),
        dispatch_handle(Arc::clone(&dispatch)),
    );
    let mut input = support::input(
        Arc::clone(&logging_obj),
        Arc::clone(&support::handlers().providers),
        EndpointType::Tinyfish,
    );
    input.raw_bytes = vec![b"data: {\"run_id\":\"run-1\"}\n\n".to_vec()];
    let tinyfish = Arc::new(FakeTinyfish::for_run(true, Some("run-1".to_owned())));
    let billing = billing_handle(Arc::clone(&tinyfish));

    route_stream(input, probe(ProbeStream::NoMessageStop), billing).await;

    assert_eq!(*tinyfish.poller_checks.lock().unwrap(), 1);
    assert!(tinyfish.scanned.lock().unwrap().is_empty());
    assert!(tinyfish.started.lock().unwrap().is_empty());
    assert!(tinyfish.handler_calls.lock().unwrap().is_empty());
    assert!(dispatch.calls().is_empty());
}

#[tokio::test]
async fn tinyfish_route_starts_billing_for_a_late_run_id() {
    let dispatch = Arc::new(RecordingDispatch::default());
    let logging_obj = object(
        state(PASS_THROUGH, &[]),
        dispatch_handle(Arc::clone(&dispatch)),
    );
    let mut input = support::input(
        Arc::clone(&logging_obj),
        Arc::clone(&support::handlers().providers),
        EndpointType::Tinyfish,
    );
    input.raw_bytes = vec![b"data: {\"run_".to_vec(), b"id\":\"run-9\"}".to_vec()];
    let tinyfish = Arc::new(FakeTinyfish::for_run(false, Some("run-9".to_owned())));
    let billing = billing_handle(Arc::clone(&tinyfish));

    route_stream(input, probe(ProbeStream::NoMessageStop), billing).await;

    assert_eq!(
        tinyfish.scanned.lock().unwrap().as_slice(),
        [b"data: {\"run_id\":\"run-9\"}".to_vec()]
    );
    assert_eq!(
        tinyfish.started.lock().unwrap().as_slice(),
        [("run-9".to_owned(), time("2024-01-01T00:00:00+00:00"))]
    );
    assert!(tinyfish.handler_calls.lock().unwrap().is_empty());
    assert!(dispatch.calls().is_empty());
}

#[tokio::test]
async fn tinyfish_route_dispatches_the_handler_result_and_kwargs() {
    let dispatch = Arc::new(RecordingDispatch::default());
    let logging_obj = object(
        state(PASS_THROUGH, &[]),
        dispatch_handle(Arc::clone(&dispatch)),
    );
    let mut input = support::input(
        Arc::clone(&logging_obj),
        Arc::clone(&support::handlers().providers),
        EndpointType::Tinyfish,
    );
    input.raw_bytes = vec![b"data: one\ndata: two\n\n".to_vec()];
    let tinyfish =
        Arc::new(FakeTinyfish::for_run(false, None).with_reply(Ok(outcome_with_kwargs())));
    let billing = billing_handle(Arc::clone(&tinyfish));

    route_stream(input, probe(ProbeStream::NoMessageStop), billing).await;

    assert_eq!(
        tinyfish.handler_calls.lock().unwrap().as_slice(),
        [vec!["data: one".to_owned(), "data: two".to_owned()]]
    );
    let calls = dispatch.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].result, response().into_json());
    assert_eq!(
        calls[0].kwargs.entries.get("model"),
        Some(&Json::Text("gpt-4o".to_owned()))
    );
    assert_eq!(calls[0].start_time, time("2024-01-01T00:00:00+00:00"));
    assert_eq!(calls[0].end_time, time("2024-01-01T00:00:02+00:00"));
}

#[tokio::test]
async fn tinyfish_route_dispatches_with_cache_hit_from_the_state() {
    let dispatch = Arc::new(RecordingDispatch::default());
    let logging_obj = object(
        state(PASS_THROUGH, &[("cache_hit", Json::Bool(true))]),
        dispatch_handle(Arc::clone(&dispatch)),
    );
    let input = support::input(
        Arc::clone(&logging_obj),
        Arc::clone(&support::handlers().providers),
        EndpointType::Tinyfish,
    );
    let tinyfish =
        Arc::new(FakeTinyfish::for_run(false, None).with_reply(Ok(outcome_with_kwargs())));
    let billing = billing_handle(Arc::clone(&tinyfish));

    route_stream(input, probe(ProbeStream::NoMessageStop), billing).await;

    assert!(dispatch.calls()[0].cache_hit);
}

#[tokio::test]
async fn tinyfish_route_logs_and_swallows_a_handler_error() {
    let (captured, _guard) = capture_logs();
    let dispatch = Arc::new(RecordingDispatch::default());
    let logging_obj = object(
        state(PASS_THROUGH, &[]),
        dispatch_handle(Arc::clone(&dispatch)),
    );
    let input = support::input(
        Arc::clone(&logging_obj),
        Arc::clone(&support::handlers().providers),
        EndpointType::Tinyfish,
    );
    let tinyfish = Arc::new(
        FakeTinyfish::for_run(false, None).with_reply(Err(HandlerError::leaf(TestLeafError))),
    );
    let billing = billing_handle(Arc::clone(&tinyfish));

    route_stream(input, probe(ProbeStream::NoMessageStop), billing).await;

    let logs = captured.text();
    assert!(logs.contains("Error in _route_streaming_logging_to_handler: the leaf failed"));
    assert!(dispatch.calls().is_empty());
}

#[tokio::test]
async fn route_dispatch_calls_dispatch_once_with_the_build_result() {
    let dispatch = Arc::new(RecordingDispatch::default());
    let logging_obj = object(
        state(PASS_THROUGH, &[]),
        dispatch_handle(Arc::clone(&dispatch)),
    );
    let handler: Arc<dyn CollectedChunksHandler> =
        Arc::new(FakeHandler::returning("openai", Ok(outcome_with_kwargs())));
    let mut input = support::input(
        Arc::clone(&logging_obj),
        providers_with(&handler),
        EndpointType::OpenAi,
    );
    input.start_time = time("2024-01-01T00:00:01+00:00");
    input.end_time = time("2024-01-01T00:00:05+00:00");
    input.raw_bytes = vec![b"data: {}\n\n".to_vec()];

    route_default(input, probe(ProbeStream::NoMessageStop)).await;

    let calls = dispatch.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].result, response().into_json());
    assert_eq!(calls[0].start_time, time("2024-01-01T00:00:01+00:00"));
    assert_eq!(calls[0].end_time, time("2024-01-01T00:00:05+00:00"));
    assert_eq!(
        calls[0].kwargs.entries.get("model"),
        Some(&Json::Text("gpt-4o".to_owned()))
    );
}

#[rstest]
#[case::json_true(Json::Bool(true), true)]
#[case::json_false(Json::Bool(false), false)]
#[case::json_string(Json::Text("true".to_owned()), false)]
#[case::json_number(json("1"), false)]
#[case::absent(Json::Null, false)]
#[tokio::test]
async fn route_dispatch_reads_cache_hit_from_a_json_true_only(
    #[case] value: Json,
    #[case] expected: bool,
) {
    let mut details: Vec<(&str, Json)> = Vec::new();
    if value != Json::Null {
        details.push(("cache_hit", value));
    }
    let dispatch = Arc::new(RecordingDispatch::default());
    let logging_obj = object(
        state(PASS_THROUGH, &details),
        dispatch_handle(Arc::clone(&dispatch)),
    );
    let handler: Arc<dyn CollectedChunksHandler> =
        Arc::new(FakeHandler::returning("openai", Ok(outcome_with_kwargs())));
    let input = support::input(
        Arc::clone(&logging_obj),
        providers_with(&handler),
        EndpointType::OpenAi,
    );

    route_default(input, probe(ProbeStream::NoMessageStop)).await;

    assert_eq!(dispatch.calls()[0].cache_hit, expected);
}

#[tokio::test]
async fn route_dispatch_builds_off_the_async_runtime() {
    let handler = Arc::new(FakeHandler::returning("openai", Ok(outcome_with_kwargs())));
    let shared: Arc<dyn CollectedChunksHandler> = handler_handle(Arc::clone(&handler));
    let logging_obj = object(
        state(PASS_THROUGH, &[]),
        Arc::new(RecordingDispatch::default()),
    );
    let input = support::input(
        Arc::clone(&logging_obj),
        providers_with(&shared),
        EndpointType::OpenAi,
    );
    let caller = current().id();

    route_default(input, probe(ProbeStream::NoMessageStop)).await;

    assert_ne!(handler.calls()[0].thread, caller);
}

#[tokio::test]
async fn route_dispatch_prefers_async_handlers_and_forwards_the_kwargs() {
    let logger = Arc::new(RecordingLogger::default());
    let dispatch = Arc::new(SuccessHandlerDispatch {
        callbacks: support::callbacks(Arc::clone(&logger), CallbackRegistry::new(16)),
    });
    let logging_obj = object(state("completion", &[]), dispatch);
    let handler: Arc<dyn CollectedChunksHandler> = Arc::new(FakeHandler::returning(
        "openai",
        Ok(HandlerOutcome {
            result: Some(response()),
            kwargs: standard_logging_kwargs(),
        }),
    ));
    let mut input = support::input(
        Arc::clone(&logging_obj),
        providers_with(&handler),
        EndpointType::OpenAi,
    );
    input.end_time = time("2024-01-01T00:00:07+00:00");

    route_default(input, probe(ProbeStream::NoMessageStop)).await;

    assert_eq!(
        logger.calls.lock().unwrap().as_slice(),
        ["async_stream".to_owned()]
    );
    let state = object_state(&logging_obj);
    assert_eq!(state.get("cache_hit"), Some(&Json::Bool(false)));
    assert_eq!(
        state.get("end_time"),
        Some(&Json::Text("2024-01-01T00:00:07+00:00".to_owned()))
    );
    assert!(state.contains_key(STANDARD_LOGGING_OBJECT));
}

#[tokio::test]
async fn route_error_logs_a_handler_failure_and_skips_dispatch() {
    let (captured, _guard) = capture_logs();
    let dispatch = Arc::new(RecordingDispatch::default());
    let logging_obj = object(
        state(PASS_THROUGH, &[]),
        dispatch_handle(Arc::clone(&dispatch)),
    );
    let handler: Arc<dyn CollectedChunksHandler> = Arc::new(FakeHandler::returning(
        "openai",
        Err(HandlerError::leaf(TestLeafError)),
    ));
    let input = support::input(
        Arc::clone(&logging_obj),
        providers_with(&handler),
        EndpointType::OpenAi,
    );

    route_default(input, probe(ProbeStream::NoMessageStop)).await;

    assert!(
        captured
            .text()
            .contains("Error in _route_streaming_logging_to_handler: the leaf failed")
    );
    assert!(dispatch.calls().is_empty());
}

#[tokio::test]
async fn route_error_logs_a_panicking_build_task_and_skips_dispatch() {
    let (captured, _guard) = capture_logs();
    let dispatch = Arc::new(RecordingDispatch::default());
    let logging_obj = object(
        state(PASS_THROUGH, &[]),
        dispatch_handle(Arc::clone(&dispatch)),
    );
    let handler: Arc<dyn CollectedChunksHandler> = Arc::new(PanickingHandler);
    let input = support::input(
        Arc::clone(&logging_obj),
        providers_with(&handler),
        EndpointType::OpenAi,
    );

    route_default(input, probe(ProbeStream::NoMessageStop)).await;

    let logs = captured.text();
    assert!(logs.contains("Error in _route_streaming_logging_to_handler:"));
    assert!(logs.contains("panicked"));
    assert!(dispatch.calls().is_empty());
}

#[derive(Debug, thiserror::Error)]
#[error("the leaf failed")]
struct TestLeafError;
