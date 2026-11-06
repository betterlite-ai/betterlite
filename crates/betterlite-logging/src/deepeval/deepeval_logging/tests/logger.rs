use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use super::super::api::ApiError;
use super::super::environment::EnvironmentError;
use super::super::payload::CallbackKwargs;
use super::super::types::Environment;
use super::super::{Call, DeepEvalError, DeepEvalLogger};
use crate::deepeval::deepeval_logging::test_support::{
    Answer, CountingUuids, Recorded, Recording, RecordingError, at,
};

const START: &str = "2024-05-01T10:00:00.120Z";
const END: &str = "2024-05-01T10:00:01.500Z";

const SUCCESS_KWARGS: &str = r#"{
    "input": [{"role": "user", "content": "hi"}],
    "standard_logging_object": {
        "id": "span-id",
        "trace_id": "trace-id",
        "model": "gpt-4o",
        "response": {
            "usage": {"prompt_tokens": 3, "completion_tokens": 5},
            "choices": [{"message": {"content": "hello"}}]
        }
    }
}"#;

const FAILURE_KWARGS: &str = r#"{
    "standard_logging_object": {
        "id": "span-id",
        "trace_id": "trace-id",
        "model": "gpt-4o",
        "error_string": "upstream returned 500",
        "response": {"usage": {"prompt_tokens": 3, "completion_tokens": 5}}
    }
}"#;

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn kwargs(text: &str) -> CallbackKwargs {
    serde_json::from_str(text).unwrap()
}

fn variables(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn logger<A>(answers: A) -> (DeepEvalLogger<Recording, CountingUuids>, Recording)
where
    A: IntoIterator<Item = Answer>,
{
    let delivery = Recording::with_answers(answers);
    let environment = variables(&[
        ("CONFIDENT_API_KEY", "confident-key"),
        ("LITELM_ENVIRONMENT", "staging"),
    ]);
    let logger =
        DeepEvalLogger::from_environment(&environment, delivery.clone(), CountingUuids::default())
            .unwrap();
    (logger, delivery)
}

fn recorded(delivery: &Recording) -> Vec<Recorded> {
    delivery.requests.lock().unwrap().clone()
}

fn call(kwargs: &CallbackKwargs) -> Call<'_> {
    Call {
        kwargs,
        start_time: at(START),
        end_time: at(END),
    }
}

fn success_body() -> Json {
    json(
        r#"{
            "uuid": "trace-id",
            "baseSpans": [],
            "agentSpans": [],
            "llmSpans": [{
                "uuid": "span-id",
                "name": "litellm_success_callback",
                "status": "SUCCESS",
                "type": "llm",
                "traceUuid": "trace-id",
                "startTime": "2024-05-01T10:00:00.120Z",
                "endTime": "2024-05-01T10:00:01.500Z",
                "input": [{"role": "user", "content": "hi"}],
                "output": "hello",
                "model": "gpt-4o",
                "inputTokenCount": 3,
                "outputTokenCount": 5
            }],
            "retrieverSpans": [],
            "toolSpans": [],
            "startTime": "2024-05-01T10:00:00.120Z",
            "endTime": "2024-05-01T10:00:01.500Z",
            "environment": "staging"
        }"#,
    )
}

fn failure_body() -> Json {
    json(
        r#"{
            "uuid": "trace-id",
            "baseSpans": [],
            "agentSpans": [],
            "llmSpans": [{
                "uuid": "span-id",
                "name": "litellm_failure_callback",
                "status": "ERRORED",
                "type": "llm",
                "traceUuid": "trace-id",
                "startTime": "2024-05-01T10:00:00.120Z",
                "endTime": "2024-05-01T10:00:01.500Z",
                "input": "NO_INPUT",
                "output": "upstream returned 500",
                "model": "gpt-4o"
            }],
            "retrieverSpans": [],
            "toolSpans": [],
            "startTime": "2024-05-01T10:00:00.120Z",
            "endTime": "2024-05-01T10:00:01.500Z",
            "environment": "staging"
        }"#,
    )
}

#[tokio::test]
async fn sync_success_event_posts_one_success_trace() {
    let (logger, delivery) = logger([Answer::status(200)]);
    let kwargs = kwargs(SUCCESS_KWARGS);
    logger.log_success_event(call(&kwargs)).await.unwrap();
    let requests = recorded(&delivery);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body, success_body());
}

#[tokio::test]
async fn sync_failure_event_posts_one_errored_trace() {
    let (logger, delivery) = logger([Answer::status(200)]);
    let kwargs = kwargs(FAILURE_KWARGS);
    logger.log_failure_event(call(&kwargs)).await.unwrap();
    let requests = recorded(&delivery);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body, failure_body());
}

#[tokio::test]
async fn async_success_event_posts_one_success_trace() {
    let (logger, delivery) = logger([Answer::status(200)]);
    let kwargs = kwargs(SUCCESS_KWARGS);
    logger.async_log_success_event(call(&kwargs)).await.unwrap();
    let requests = recorded(&delivery);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body, success_body());
}

#[tokio::test]
async fn async_failure_event_posts_one_errored_trace() {
    let (logger, delivery) = logger([Answer::status(200)]);
    let kwargs = kwargs(FAILURE_KWARGS);
    logger.async_log_failure_event(call(&kwargs)).await.unwrap();
    let requests = recorded(&delivery);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body, failure_body());
}

#[tokio::test]
async fn every_event_posts_to_the_confident_tracing_endpoint_with_the_key() {
    let (logger, delivery) = logger([Answer::status(200)]);
    let kwargs = kwargs(SUCCESS_KWARGS);
    logger.log_success_event(call(&kwargs)).await.unwrap();
    let requests = recorded(&delivery);
    assert_eq!(requests[0].url, "https://api.confident-ai.com/v1/tracing");
    assert_eq!(requests[0].headers["confident_api_key"], "confident-key");
    assert_eq!(requests[0].headers["content-type"], "application/json");
}

#[tokio::test]
async fn missing_identifiers_fall_back_to_generated_uuids_shared_by_span_and_trace() {
    let (logger, delivery) = logger([Answer::status(200)]);
    let kwargs = kwargs(
        r#"{"standard_logging_object": {"response": {"choices": [{"message": {"content": "hello"}}]}}}"#,
    );
    logger.log_success_event(call(&kwargs)).await.unwrap();
    let requests = recorded(&delivery);
    let Json::Object(fields) = &requests[0].body else {
        panic!("the trace is not an object");
    };
    assert_eq!(fields["uuid"], json(r#""generated-1""#));
    let Json::List(spans) = &fields["llmSpans"] else {
        panic!("llmSpans is not a list");
    };
    let Json::Object(span) = &spans[0] else {
        panic!("the span is not an object");
    };
    assert_eq!(span["uuid"], json(r#""generated-0""#));
    assert_eq!(span["traceUuid"], json(r#""generated-1""#));
}

#[tokio::test]
async fn a_tool_call_response_leaves_the_output_out() {
    let (logger, delivery) = logger([Answer::status(200)]);
    let kwargs = kwargs(
        r#"{
            "standard_logging_object": {
                "id": "span-id",
                "trace_id": "trace-id",
                "response": {"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [{"id": "call-1"}]}}]}
            }
        }"#,
    );
    logger.log_success_event(call(&kwargs)).await.unwrap();
    let requests = recorded(&delivery);
    let Json::Object(fields) = &requests[0].body else {
        panic!("the trace is not an object");
    };
    let Json::List(spans) = &fields["llmSpans"] else {
        panic!("llmSpans is not a list");
    };
    let Json::Object(span) = &spans[0] else {
        panic!("the span is not an object");
    };
    assert!(!span.contains_key("output"));
    assert_eq!(span["input"], json(r#""NO_INPUT""#));
}

#[test]
fn environment_defaults_to_development() {
    let environment = variables(&[("CONFIDENT_API_KEY", "confident-key")]);
    let logger = DeepEvalLogger::from_environment(
        &environment,
        Recording::default(),
        CountingUuids::default(),
    )
    .unwrap();
    assert_eq!(logger.environment, Environment::Development);
}

#[test]
fn litelm_environment_is_read_with_its_misspelling() {
    let environment = variables(&[
        ("CONFIDENT_API_KEY", "confident-key"),
        ("LITELM_ENVIRONMENT", "production"),
    ]);
    let logger = DeepEvalLogger::from_environment(
        &environment,
        Recording::default(),
        CountingUuids::default(),
    )
    .unwrap();
    assert_eq!(logger.environment, Environment::Production);
}

#[test]
fn the_correctly_spelled_environment_variable_is_ignored() {
    let environment = variables(&[
        ("CONFIDENT_API_KEY", "confident-key"),
        ("LITELLM_ENVIRONMENT", "production"),
    ]);
    let logger = DeepEvalLogger::from_environment(
        &environment,
        Recording::default(),
        CountingUuids::default(),
    )
    .unwrap();
    assert_eq!(logger.environment, Environment::Development);
}

#[test]
fn missing_confident_api_key_is_an_error() {
    let environment = variables(&[("LITELM_ENVIRONMENT", "staging")]);
    let result = DeepEvalLogger::from_environment(
        &environment,
        Recording::default(),
        CountingUuids::default(),
    );
    assert!(matches!(result, Err(DeepEvalError::MissingApiKey)));
}

#[test]
fn an_empty_confident_api_key_counts_as_unset() {
    let environment = variables(&[("CONFIDENT_API_KEY", "")]);
    let result = DeepEvalLogger::from_environment(
        &environment,
        Recording::default(),
        CountingUuids::default(),
    );
    assert!(matches!(result, Err(DeepEvalError::MissingApiKey)));
}

#[test]
fn an_invalid_environment_is_rejected_before_the_missing_key() {
    let environment = variables(&[("LITELM_ENVIRONMENT", "qa")]);
    let result = DeepEvalLogger::from_environment(
        &environment,
        Recording::default(),
        CountingUuids::default(),
    );
    let Err(DeepEvalError::Environment(EnvironmentError::Invalid {
        environment: invalid,
    })) = result
    else {
        panic!("an unknown environment must be rejected");
    };
    assert_eq!(invalid, "qa");
}

#[tokio::test]
async fn sync_event_raises_on_a_transport_failure() {
    let (logger, _delivery) = logger([Answer::Fail]);
    let kwargs = kwargs(SUCCESS_KWARGS);
    assert!(matches!(
        logger.log_success_event(call(&kwargs)).await,
        Err(DeepEvalError::Api(ApiError::Transport(RecordingError)))
    ));
}

#[tokio::test]
async fn sync_event_raises_on_a_non_200_status() {
    let (logger, _delivery) = logger([Answer::reply(500, r#"{"error": "boom"}"#)]);
    let kwargs = kwargs(SUCCESS_KWARGS);
    let result = logger.log_success_event(call(&kwargs)).await;
    let Err(DeepEvalError::Api(ApiError::Status { status, body })) = result else {
        panic!("a 500 must not be accepted");
    };
    assert_eq!(status, 500);
    assert_eq!(body, r#"{"error": "boom"}"#);
}

#[tokio::test]
async fn async_event_raises_on_a_non_200_status() {
    let (logger, _delivery) = logger([Answer::reply(503, "unavailable")]);
    let kwargs = kwargs(FAILURE_KWARGS);
    let result = logger.async_log_failure_event(call(&kwargs)).await;
    let Err(DeepEvalError::Api(ApiError::Status { status, body })) = result else {
        panic!("a 503 must not be accepted");
    };
    assert_eq!(status, 503);
    assert_eq!(body, "unavailable");
}
