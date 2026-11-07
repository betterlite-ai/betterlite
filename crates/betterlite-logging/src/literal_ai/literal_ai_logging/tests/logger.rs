#![cfg(test)]

use std::iter::repeat_with;

use betterlite_models::decisions::Json;

use super::super::payload::CallbackKwargs;
use super::super::prepare::prepare_log_data;
use super::super::send::SendError;
use super::super::settings::{Overrides, Settings};
use super::super::{Call, LiteralAILogger};
use super::support::{
    Answer, Buffer, FixedIds, Recording, at, capture, json, kwargs, object, text, variables,
};
use crate::batch::logger::{BatchQueue, DEFAULT_MAX_QUEUE_SIZE};

const START: &str = "2024-05-01T10:00:00.120Z";
const END: &str = "2024-05-01T10:00:01.500Z";

const CALLBACK: &str = r#"{
    "model": "gpt-4o",
    "custom_llm_provider": "openai",
    "standard_logging_object": {
        "metadata": {"user_api_key": "hashed"},
        "model_parameters": {"model": "gpt-4o"},
        "messages": [{"role": "user", "content": "hi"}],
        "response": {"choices": [{"message": {"role": "assistant", "content": "hello"}}]},
        "error_str": "upstream said no",
        "prompt_tokens": 3,
        "completion_tokens": 5,
        "total_tokens": 8
    }
}"#;

const WITHOUT_PAYLOAD: &str = r#"{"model": "gpt-4o"}"#;

type Logger = LiteralAILogger<Recording, FixedIds>;

fn logger(environment: &[(&str, &str)], answers: Vec<Answer>) -> (Logger, Recording) {
    let delivery = Recording::with_answers(answers);
    let logger = LiteralAILogger::from_environment(
        &variables(environment),
        Overrides::default(),
        delivery.clone(),
        FixedIds::default(),
    )
    .unwrap();
    (logger, delivery)
}

fn batch_logger(answers: Vec<Answer>) -> (Logger, Recording) {
    logger(
        &[
            ("LITERAL_API_KEY", "literal-key"),
            ("LITERAL_BATCH_SIZE", "2"),
        ],
        answers,
    )
}

fn call(kwargs: &CallbackKwargs) -> Call<'_> {
    Call {
        kwargs,
        start_time: at(START),
        end_time: at(END),
    }
}

#[tokio::test]
async fn log_success_event_queues_below_the_batch_size() {
    let (logger, delivery) = batch_logger(Vec::new());
    let kwargs = kwargs(CALLBACK);
    logger.log_success_event(call(&kwargs)).await;
    assert_eq!(logger.queue.pending().len(), 1);
    assert!(delivery.requests().is_empty());
}

#[tokio::test]
async fn log_success_event_sends_at_the_batch_size_without_clearing_the_queue() {
    let (logger, delivery) = batch_logger(Vec::new());
    let kwargs = kwargs(CALLBACK);
    logger.log_success_event(call(&kwargs)).await;
    logger.log_success_event(call(&kwargs)).await;
    let requests = delivery.requests();
    assert_eq!(requests.len(), 1);
    let variables = object(&object(&requests[0].body)["variables"]);
    assert_eq!(variables["id_0"], Json::Text("generated-0".to_owned()));
    assert_eq!(variables["id_1"], Json::Text("generated-1".to_owned()));
    assert_eq!(logger.queue.pending().len(), 2);
}

#[tokio::test]
async fn log_failure_event_sends_at_the_batch_size_without_clearing_the_queue() {
    let (logger, delivery) = batch_logger(Vec::new());
    let kwargs = kwargs(CALLBACK);
    logger.log_failure_event(call(&kwargs)).await;
    logger.log_failure_event(call(&kwargs)).await;
    let requests = delivery.requests();
    assert_eq!(requests.len(), 1);
    let variables = object(&object(&requests[0].body)["variables"]);
    assert_eq!(
        variables["error_0"],
        Json::Text("upstream said no".to_owned())
    );
    assert_eq!(logger.queue.pending().len(), 2);
}

#[tokio::test]
async fn a_third_event_resends_the_whole_sync_queue() {
    let (logger, delivery) = batch_logger(Vec::new());
    let kwargs = kwargs(CALLBACK);
    logger.log_success_event(call(&kwargs)).await;
    logger.log_success_event(call(&kwargs)).await;
    logger.log_success_event(call(&kwargs)).await;
    let requests = delivery.requests();
    assert_eq!(requests.len(), 2);
    let variables = object(&object(&requests[1].body)["variables"]);
    assert!(variables.contains_key("id_0"));
    assert!(variables.contains_key("id_1"));
    assert!(variables.contains_key("id_2"));
}

#[tokio::test]
async fn async_log_success_event_flushes_the_queue_at_the_batch_size() {
    let (logger, delivery) = batch_logger(Vec::new());
    let kwargs = kwargs(CALLBACK);
    logger.async_log_success_event(call(&kwargs)).await;
    assert!(delivery.requests().is_empty());
    assert_eq!(logger.queue.pending().len(), 1);
    logger.async_log_success_event(call(&kwargs)).await;
    assert_eq!(delivery.requests().len(), 1);
    assert!(logger.queue.pending().is_empty());
}

#[tokio::test]
async fn an_empty_queue_sends_no_request() {
    let (logger, delivery) = batch_logger(Vec::new());
    logger.send_batch().await.unwrap();
    logger.flush_queue().await;
    assert!(delivery.requests().is_empty());
}

#[tokio::test]
async fn async_log_failure_event_flushes_the_queue_at_the_batch_size() {
    let (logger, delivery) = batch_logger(Vec::new());
    let kwargs = kwargs(CALLBACK);
    logger.async_log_failure_event(call(&kwargs)).await;
    logger.async_log_failure_event(call(&kwargs)).await;
    assert_eq!(delivery.requests().len(), 1);
    assert!(logger.queue.pending().is_empty());
}

#[tokio::test]
async fn send_batch_posts_to_the_graphql_endpoint_with_the_client_headers() {
    let (logger, delivery) = batch_logger(Vec::new());
    let kwargs = kwargs(CALLBACK);
    logger.log_success_event(call(&kwargs)).await;
    logger.send_batch().await.unwrap();
    let requests = delivery.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.url, "https://cloud.getliteral.ai/api/graphql");
    assert_eq!(request.headers["content-type"], "application/json");
    assert_eq!(request.headers["x-client-name"], "litellm");
    assert_eq!(request.headers["x-api-key"], "literal-key");
    assert!(!request.headers.contains_key("x-env"));
}

#[tokio::test]
async fn the_request_goes_to_the_literal_api_url_from_the_environment() {
    let (logger, delivery) = logger(
        &[
            ("LITERAL_API_KEY", "literal-key"),
            ("LITERAL_API_URL", "https://literal.internal"),
        ],
        Vec::new(),
    );
    let kwargs = kwargs(CALLBACK);
    logger.log_success_event(call(&kwargs)).await;
    logger.send_batch().await.unwrap();
    assert_eq!(
        delivery.requests()[0].url,
        "https://literal.internal/api/graphql"
    );
}

#[tokio::test]
async fn the_request_goes_to_the_constructor_url_when_the_environment_has_none() {
    let delivery = Recording::with_answers(Vec::new());
    let environment = variables(&[("LITERAL_API_KEY", "literal-key")]);
    let configured = LiteralAILogger::from_environment(
        &environment,
        Overrides {
            api_key: None,
            api_url: Some("https://constructor.local".to_owned()),
            environment: None,
        },
        delivery.clone(),
        FixedIds::default(),
    )
    .unwrap();
    let kwargs = kwargs(CALLBACK);
    configured.log_success_event(call(&kwargs)).await;
    configured.send_batch().await.unwrap();
    assert_eq!(
        delivery.requests()[0].url,
        "https://constructor.local/api/graphql"
    );
}

#[tokio::test]
async fn a_batch_size_above_the_queue_cap_still_sends() {
    let mut settings = Settings::from_environment(
        &variables(&[
            ("LITERAL_API_KEY", "literal-key"),
            ("LITERAL_BATCH_SIZE", "50"),
        ]),
        Overrides::default(),
    )
    .unwrap();
    settings.batch.max_queue_size = 2;
    let delivery = Recording::with_answers(Vec::new());
    let capped = LiteralAILogger {
        queue: BatchQueue::new(settings.batch),
        settings,
        delivery: delivery.clone(),
        ids: FixedIds::default(),
    };
    let kwargs = kwargs(CALLBACK);
    capped.log_success_event(call(&kwargs)).await;
    assert!(delivery.requests().is_empty());
    capped.log_success_event(call(&kwargs)).await;
    assert_eq!(delivery.requests().len(), 1);
    assert_eq!(capped.queue.pending().len(), 2);
}

#[tokio::test]
async fn the_request_body_carries_the_query_and_the_step_variables() {
    let (logger, delivery) = batch_logger(Vec::new());
    let kwargs = kwargs(CALLBACK);
    logger.log_success_event(call(&kwargs)).await;
    logger.send_batch().await.unwrap();
    let body = object(&delivery.requests()[0].body);
    assert!(text(&body["query"]).starts_with("\n        mutation AddStep($id_0: String!"));
    let variables = object(&body["variables"]);
    assert_eq!(variables["name_0"], Json::Text("gpt-4o".to_owned()));
    assert_eq!(variables["type_0"], Json::Text("llm".to_owned()));
    assert_eq!(
        variables["metadata_0"],
        json(r#"{"user_api_key": "hashed"}"#)
    );
}

#[tokio::test]
async fn send_batch_keeps_the_queue_after_a_server_error() {
    let (logger, delivery) = batch_logger(vec![Answer::status(500)]);
    let kwargs = kwargs(CALLBACK);
    logger.log_success_event(call(&kwargs)).await;
    let error = logger.send_batch().await.unwrap_err();
    assert!(matches!(error, SendError::Status { status: 500, .. }));
    assert_eq!(logger.queue.pending().len(), 1);
    assert_eq!(delivery.requests().len(), 1);
}

#[tokio::test]
async fn send_batch_keeps_the_queue_after_a_transport_error() {
    let (logger, _delivery) = batch_logger(vec![Answer::Fail]);
    let kwargs = kwargs(CALLBACK);
    logger.log_success_event(call(&kwargs)).await;
    let error = logger.send_batch().await.unwrap_err();
    assert!(matches!(error, SendError::Transport(_)));
    assert_eq!(logger.queue.pending().len(), 1);
}

#[tokio::test]
async fn async_flush_clears_the_queue_after_a_server_error() {
    let buffer = Buffer::default();
    let _guard = capture(&buffer);
    let (logger, delivery) = batch_logger(vec![Answer::reply(503, "maintenance")]);
    let kwargs = kwargs(CALLBACK);
    logger.async_log_success_event(call(&kwargs)).await;
    logger.async_log_success_event(call(&kwargs)).await;
    assert!(logger.queue.pending().is_empty());
    assert_eq!(delivery.requests().len(), 1);
    assert!(
        buffer
            .contents()
            .contains("Literal AI HTTP Error: 503 - maintenance")
    );
}

#[tokio::test]
async fn async_flush_clears_the_queue_after_a_transport_error() {
    let (logger, delivery) = batch_logger(vec![Answer::Fail]);
    let kwargs = kwargs(CALLBACK);
    logger.async_log_failure_event(call(&kwargs)).await;
    logger.async_log_failure_event(call(&kwargs)).await;
    assert!(logger.queue.pending().is_empty());
    assert_eq!(delivery.requests().len(), 1);
}

#[tokio::test]
async fn send_batch_without_an_api_key_sends_nothing() {
    let (logger, delivery) = logger(&[("LITERAL_BATCH_SIZE", "2")], Vec::new());
    let kwargs = kwargs(CALLBACK);
    logger.log_success_event(call(&kwargs)).await;
    let error = logger.send_batch().await.unwrap_err();
    assert!(matches!(error, SendError::MissingApiKey));
    assert!(delivery.requests().is_empty());
    assert_eq!(logger.queue.pending().len(), 1);
}

#[tokio::test]
async fn a_successful_batch_is_logged_with_its_size() {
    let buffer = Buffer::default();
    let _guard = capture(&buffer);
    let (logger, _delivery) = batch_logger(Vec::new());
    let kwargs = kwargs(CALLBACK);
    logger.log_success_event(call(&kwargs)).await;
    logger.log_success_event(call(&kwargs)).await;
    assert!(
        buffer
            .contents()
            .contains("Batch of 2 runs successfully created")
    );
}

#[tokio::test]
async fn the_sync_callback_logs_a_server_error_with_its_body() {
    let buffer = Buffer::default();
    let _guard = capture(&buffer);
    let (logger, _delivery) = batch_logger(vec![Answer::reply(502, "bad gateway")]);
    let kwargs = kwargs(CALLBACK);
    logger.log_success_event(call(&kwargs)).await;
    logger.log_success_event(call(&kwargs)).await;
    assert!(
        buffer
            .contents()
            .contains("Literal AI Error: 502 - bad gateway")
    );
}

#[tokio::test]
async fn a_prepare_failure_is_logged_and_leaves_the_queue_empty() {
    let buffer = Buffer::default();
    let _guard = capture(&buffer);
    let (logger, delivery) = batch_logger(Vec::new());
    let kwargs = kwargs(WITHOUT_PAYLOAD);
    logger.log_success_event(call(&kwargs)).await;
    assert!(logger.queue.pending().is_empty());
    assert!(delivery.requests().is_empty());
    assert!(buffer.contents().contains("error logging success event"));
}

#[test]
fn the_sync_queue_keeps_the_maximum_and_drops_the_oldest() {
    let (logger, _delivery) = batch_logger(Vec::new());
    let step = prepare_log_data(
        &kwargs(r#"{"standard_logging_object": {"model_parameters": {}, "response": {}}}"#),
        at(START),
        at(END),
        &FixedIds::default(),
    )
    .unwrap();
    let outcomes: Vec<_> = repeat_with(|| logger.queue.enqueue(step.clone()))
        .take(DEFAULT_MAX_QUEUE_SIZE.checked_add(1).unwrap())
        .collect();
    let last = &outcomes[DEFAULT_MAX_QUEUE_SIZE];
    assert_eq!(last.depth, DEFAULT_MAX_QUEUE_SIZE);
    assert_eq!(last.dropped_oldest, 1);
    assert_eq!(logger.queue.pending().len(), DEFAULT_MAX_QUEUE_SIZE);
}

#[tokio::test]
async fn the_failure_callback_logs_its_event_line() {
    let buffer = Buffer::default();
    let _guard = capture(&buffer);
    let (logger, _delivery) = batch_logger(Vec::new());
    let kwargs = kwargs(CALLBACK);
    logger.log_failure_event(call(&kwargs)).await;
    assert!(
        buffer
            .contents()
            .contains("Literal AI Failure Event Logging!")
    );
}
