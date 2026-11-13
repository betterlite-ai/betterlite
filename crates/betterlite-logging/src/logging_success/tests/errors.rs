use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{
    CallLog, FakeLogger, Fixture, chat_result, correlation, payload, request, state,
};
use crate::logging_success::dispatch::dispatch_success_handlers;
use crate::logging_success::payload_state::standard_logging_object;

#[rstest]
#[case::sync_failure(false)]
#[case::async_failure(true)]
#[tokio::test]
async fn logging_success_behavior_tests_behavior(#[case] asynchronous: bool) {
    let mut fixture = Fixture::new();
    let failing_calls = Arc::new(CallLog::default());
    let mut failing_logger = FakeLogger::new("langfuse", &failing_calls);
    failing_logger.fail = true;
    let failing_handle = Arc::new(failing_logger);
    let following_calls = Arc::new(CallLog::default());
    let following = Arc::new(FakeLogger::new("datadog", &following_calls));
    fixture.add(&failing_handle);
    fixture.add(&following);
    let mut state = state(Some(false));
    if asynchronous {
        state
            .litellm_params
            .insert("acompletion".to_owned(), Json::Bool(true));
    }
    let mut correlation = correlation();
    dispatch_success_handlers(
        &fixture.handler(),
        &mut state,
        &mut correlation,
        &request(chat_result()),
    )
    .await;
    assert_eq!(failing_calls.events().len(), 1);
    assert_eq!(following_calls.events().len(), 1);
    assert_eq!(fixture.counter_calls[0].calls(), ["langfuse"]);
    assert!(fixture.counter_calls[1].calls().is_empty());
    let stored = standard_logging_object(&state).unwrap().unwrap();
    assert_eq!(stored.model, payload().model);
    assert_eq!(stored.messages, payload().messages);
}
