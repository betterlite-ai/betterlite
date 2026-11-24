use betterlite_context::non_default_params::Params;
use betterlite_models::controls::json_value::JsonValue;

use super::support::{
    AsyncRecordingOperation, FakeError, Fixture, MESSAGE_RESPONSE, RecordingOperation,
    async_stream, delta_chunk, object, params, response, sync_stream, value,
};
use crate::api::responses_handler::dispatch::{
    InteractionsAnswer, async_interactions_api_handler, interactions_api_handler,
};
use crate::api::responses_handler::error::InteractionsHandlerError;
use crate::api::responses_handler::streams::{AsyncResponsesAnswer, SyncResponsesAnswer};
use crate::api::responses_transform::request::to_responses_request;

#[test]
fn handler_returns_the_transformed_interaction_for_a_complete_response() {
    let fixture = Fixture::new("gpt-4o");
    let operation = RecordingOperation::new(Ok(SyncResponsesAnswer::Complete(Box::new(response(
        MESSAGE_RESPONSE,
    )))));
    let answer =
        interactions_api_handler(&operation, &fixture.request(), ()).expect("the handler succeeds");
    let InteractionsAnswer::Complete(interaction) = answer else {
        panic!("a complete response must not become a stream");
    };
    assert_eq!(interaction.id.as_deref(), Some("resp_1"));
    assert_eq!(interaction.model.as_deref(), Some("gpt-4o"));
    assert_eq!(interaction.created.as_deref(), Some("2023-11-14T22:13:20"));
    assert_eq!(
        interaction.steps,
        Some(vec![object(
            r#"{"type": "model_output", "content": [{"type": "text", "text": "hello"}]}"#
        )])
    );
    assert!(operation.recorder.recorded().is_some());
}

#[test]
fn handler_wraps_a_streaming_answer_in_the_iterator() {
    let fixture = Fixture::new("gpt-4o");
    let stream = sync_stream(vec![Ok(delta_chunk("item_1", "hello"))]);
    let operation = RecordingOperation::new(Ok(SyncResponsesAnswer::Stream(stream)));
    let answer =
        interactions_api_handler(&operation, &fixture.request(), ()).expect("the handler succeeds");
    let InteractionsAnswer::Stream(mut iterator) = answer else {
        panic!("a streaming answer must become an iterator");
    };
    assert_eq!(iterator.state.model, "gpt-4o");
    let created = iterator
        .next_event()
        .expect("the interaction start")
        .expect("the fake stream does not fail");
    assert_eq!(created.event_type.as_deref(), Some("interaction.created"));
}

#[test]
fn handler_builds_the_responses_request_from_the_arguments() {
    let mut fixture = Fixture::new("gpt-4o");
    fixture.input = Some(value(r#""hi""#));
    fixture.stream = Some(true);
    fixture.custom_llm_provider = Some(String::from("openai"));
    fixture.optional_params =
        params(r#"{"stream": false, "system_instruction": "be nice", "store": true}"#);
    fixture.extras = params(r#"{"litellm_metadata": {"team": "x"}}"#);
    let operation = RecordingOperation::new(Ok(SyncResponsesAnswer::Complete(Box::new(response(
        MESSAGE_RESPONSE,
    )))));
    interactions_api_handler(&operation, &fixture.request(), ()).expect("the handler succeeds");
    let recorded = operation
        .recorder
        .recorded()
        .expect("the operation is called");
    assert_eq!(recorded.model, "gpt-4o");
    assert!(recorded.input.is_some());
    assert_eq!(recorded.instructions.as_deref(), Some("be nice"));
    assert_eq!(recorded.stream, Some(true));
    assert_eq!(recorded.store, Some(true));
    assert_eq!(
        recorded.passthrough.get("custom_llm_provider"),
        Some(&JsonValue::Text(String::from("openai")))
    );
    assert_eq!(
        recorded.passthrough.get("litellm_metadata"),
        Some(&value(r#"{"team": "x"}"#))
    );
}

#[test]
fn handler_without_a_stream_flag_clears_the_optional_params_stream() {
    let mut fixture = Fixture::new("gpt-4o");
    fixture.optional_params = params(r#"{"stream": true}"#);
    let operation = RecordingOperation::new(Ok(SyncResponsesAnswer::Complete(Box::new(response(
        MESSAGE_RESPONSE,
    )))));
    interactions_api_handler(&operation, &fixture.request(), ()).expect("the handler succeeds");
    let recorded = operation
        .recorder
        .recorded()
        .expect("the operation is called");
    assert_eq!(recorded.stream, None);
    assert!(!recorded.passthrough.contains_key("custom_llm_provider"));
}

#[test]
fn handler_reports_a_response_that_cannot_be_transformed() {
    let fixture = Fixture::new("gpt-4o");
    let operation = RecordingOperation::new(Ok(SyncResponsesAnswer::Complete(Box::new(response(
        r#"{"id": "resp_1", "created_at": 9223372036854775807, "output": []}"#,
    )))));
    let Err(error) = interactions_api_handler(&operation, &fixture.request(), ()) else {
        panic!("the transform must refuse the created_at")
    };
    assert!(matches!(error, InteractionsHandlerError::Transform(_)));
}

#[test]
fn handler_reports_a_request_that_cannot_be_transformed() {
    let mut fixture = Fixture::new("gpt-4o");
    fixture.optional_params = params(r#"{"system_instruction": 5}"#);
    let operation = RecordingOperation::new(Ok(SyncResponsesAnswer::Complete(Box::new(response(
        MESSAGE_RESPONSE,
    )))));
    let Err(error) = interactions_api_handler(&operation, &fixture.request(), ()) else {
        panic!("the transform must refuse the request")
    };
    assert!(matches!(error, InteractionsHandlerError::Transform(_)));
    assert!(operation.recorder.recorded().is_none());
}

#[test]
fn handler_reports_a_failed_responses_call() {
    let fixture = Fixture::new("gpt-4o");
    let operation = RecordingOperation::new(Err(FakeError::Operation));
    let Err(error) = interactions_api_handler(&operation, &fixture.request(), ()) else {
        panic!("the failed call must be reported")
    };
    assert!(matches!(
        error,
        InteractionsHandlerError::Call(FakeError::Operation)
    ));
}

#[tokio::test]
async fn async_handler_returns_the_transformed_interaction_for_a_complete_response() {
    let fixture = Fixture::new("gpt-4o");
    let operation = AsyncRecordingOperation::new(Ok(AsyncResponsesAnswer::Complete(Box::new(
        response(MESSAGE_RESPONSE),
    ))));
    let responses_request = to_responses_request("gpt-4o", None, &Params::new(), &Params::new())
        .expect("the request should build");
    let request = fixture.async_request(&responses_request);
    let answer = async_interactions_api_handler(&operation, &request, ())
        .await
        .expect("the handler succeeds");
    let InteractionsAnswer::Complete(interaction) = answer else {
        panic!("a complete response must not become a stream");
    };
    assert_eq!(interaction.id.as_deref(), Some("resp_1"));
    assert_eq!(interaction.model.as_deref(), Some("gpt-4o"));
}

#[tokio::test]
async fn async_handler_wraps_a_streaming_answer_in_the_iterator() {
    let fixture = Fixture::new("gpt-4o");
    let stream = async_stream(vec![Ok(delta_chunk("item_1", "hello"))]);
    let operation = AsyncRecordingOperation::new(Ok(AsyncResponsesAnswer::Stream(stream)));
    let responses_request = to_responses_request("gpt-4o", None, &Params::new(), &Params::new())
        .expect("the request should build");
    let request = fixture.async_request(&responses_request);
    let answer = async_interactions_api_handler(&operation, &request, ())
        .await
        .expect("the handler succeeds");
    let InteractionsAnswer::Stream(mut iterator) = answer else {
        panic!("a streaming answer must become an iterator");
    };
    let created = iterator
        .next_event()
        .await
        .expect("the interaction start")
        .expect("the fake stream does not fail");
    assert_eq!(created.event_type.as_deref(), Some("interaction.created"));
    assert_eq!(iterator.state.custom_llm_provider, None);
}

#[tokio::test]
async fn async_handler_takes_the_provider_from_the_request() {
    let fixture = Fixture::new("gpt-4o");
    let extras = params(r#"{"custom_llm_provider": "openai"}"#);
    let responses_request = to_responses_request("gpt-4o", None, &Params::new(), &extras)
        .expect("the request should build");
    let stream = async_stream(vec![Ok(delta_chunk("item_1", "hello"))]);
    let operation = AsyncRecordingOperation::new(Ok(AsyncResponsesAnswer::Stream(stream)));
    let request = fixture.async_request(&responses_request);
    let answer = async_interactions_api_handler(&operation, &request, ())
        .await
        .expect("the handler succeeds");
    let InteractionsAnswer::Stream(iterator) = answer else {
        panic!("a streaming answer must become an iterator");
    };
    assert_eq!(
        iterator.state.custom_llm_provider.as_deref(),
        Some("openai")
    );
}

#[tokio::test]
async fn async_handler_ignores_a_provider_that_is_not_text() {
    let fixture = Fixture::new("gpt-4o");
    let extras = params(r#"{"custom_llm_provider": 5}"#);
    let responses_request = to_responses_request("gpt-4o", None, &Params::new(), &extras)
        .expect("the request should build");
    let stream = async_stream(vec![Ok(delta_chunk("item_1", "hello"))]);
    let operation = AsyncRecordingOperation::new(Ok(AsyncResponsesAnswer::Stream(stream)));
    let request = fixture.async_request(&responses_request);
    let answer = async_interactions_api_handler(&operation, &request, ())
        .await
        .expect("the handler succeeds");
    let InteractionsAnswer::Stream(iterator) = answer else {
        panic!("a streaming answer must become an iterator");
    };
    assert_eq!(iterator.state.custom_llm_provider, None);
}

#[tokio::test]
async fn async_handler_uses_the_prebuilt_request() {
    let fixture = Fixture::new("gpt-4o");
    let optional_params = params(r#"{"system_instruction": "be nice"}"#);
    let responses_request = to_responses_request("gpt-4o", None, &optional_params, &Params::new())
        .expect("the request should build");
    let operation = AsyncRecordingOperation::new(Ok(AsyncResponsesAnswer::Complete(Box::new(
        response(MESSAGE_RESPONSE),
    ))));
    let request = fixture.async_request(&responses_request);
    async_interactions_api_handler(&operation, &request, ())
        .await
        .expect("the handler succeeds");
    let recorded = operation
        .recorder
        .recorded()
        .expect("the operation is called");
    assert_eq!(recorded.instructions.as_deref(), Some("be nice"));
}

#[tokio::test]
async fn async_handler_reports_a_failed_responses_call() {
    let fixture = Fixture::new("gpt-4o");
    let operation = AsyncRecordingOperation::new(Err(FakeError::Operation));
    let responses_request = to_responses_request("gpt-4o", None, &Params::new(), &Params::new())
        .expect("the request should build");
    let request = fixture.async_request(&responses_request);
    let Err(error) = async_interactions_api_handler(&operation, &request, ()).await else {
        panic!("the failed call must be reported")
    };
    assert!(matches!(
        error,
        InteractionsHandlerError::Call(FakeError::Operation)
    ));
}
