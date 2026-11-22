use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;

use super::support::{
    Bench, MOCK_JSON, Outcome, PROVIDER, VIDEO_ID, call, failed, handles, kwargs, lookups,
    object_json, ready, request, text,
};
use crate::api::video_status_call::error::VideoStatusCallError;

#[test]
fn mock_response_json_string_answers_without_the_provider() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.custom_llm_provider = None;
    request.kwargs = kwargs(&[("mock_response", text(MOCK_JSON))]);
    let video = ready(call(&mut bench, request));
    assert_eq!(video.id, "mock-1");
    assert_eq!(video.status, "completed");
    assert!(lookups(&bench.journal).is_empty());
    assert!(handles(&bench.journal).is_empty());
}

#[test]
fn mock_response_map_answers_with_the_mock_object() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("mock_response", object_json("mock-2"))]);
    let video = ready(call(&mut bench, request));
    assert_eq!(video.id, "mock-2");
    assert_eq!(video.status, "completed");
    assert!(handles(&bench.journal).is_empty());
}

#[test]
fn mock_response_null_takes_the_provider_path() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("mock_response", Json::Null)]);
    let video = ready(call(&mut bench, request));
    assert_eq!(video.id, VIDEO_ID);
    assert_eq!(lookups(&bench.journal), vec![PROVIDER.to_owned()]);
    assert_eq!(handles(&bench.journal).len(), 1);
}

#[test]
fn mock_response_invalid_json_is_an_api_connection_error() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("mock_response", text("{not json"))]);
    let error = failed(call(&mut bench, request));
    assert_eq!(error.to_string(), "the mock response is not valid JSON");
    let VideoStatusCallError::Mapped {
        class: ErrorClass::ApiConnection(context),
        ..
    } = &error
    else {
        panic!("an invalid mock is an api connection error");
    };
    assert_eq!(
        context.message,
        "GeminiException - the mock response is not valid JSON"
    );
    assert_eq!(context.provider.as_deref(), Some(PROVIDER));
    assert!(lookups(&bench.journal).is_empty());
    assert!(handles(&bench.journal).is_empty());
}

#[test]
fn mock_response_missing_fields_is_a_model_error() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[(
        "mock_response",
        text(r#"{"object":"video","status":"completed"}"#),
    )]);
    let error = failed(call(&mut bench, request));
    assert_eq!(
        error.to_string(),
        "the mock response does not match a video object"
    );
    assert!(matches!(
        error,
        VideoStatusCallError::Mapped {
            class: ErrorClass::ApiConnection(_),
            ..
        }
    ));
    assert!(lookups(&bench.journal).is_empty());
}

#[test]
fn mock_response_non_object_value_is_a_model_error() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("mock_response", text("7"))]);
    let error = failed(call(&mut bench, request));
    assert_eq!(
        error.to_string(),
        "the mock response does not match a video object"
    );
    assert!(handles(&bench.journal).is_empty());
}

#[test]
fn mock_response_answers_even_when_async_call_is_true() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[
        ("async_call", Json::Bool(true)),
        ("mock_response", text(MOCK_JSON)),
    ]);
    let video = ready(call(&mut bench, request));
    assert_eq!(video.id, "mock-1");
    assert!(handles(&bench.journal).is_empty());
}

#[test]
fn mock_response_runs_before_the_logging_update() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("mock_response", text(MOCK_JSON))]);
    ready(call(&mut bench, request));
    assert_eq!(bench.fixture.state.model, "video-model");
    assert_eq!(bench.fixture.state.call_type, "video_generation");
    assert!(bench.fixture.state.optional_params.is_empty());
}
