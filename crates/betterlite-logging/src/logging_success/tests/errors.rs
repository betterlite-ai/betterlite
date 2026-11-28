use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{
    CallLog, FakeLogger, Fixture, RecordingCounter, chat_result, correlation, payload, request,
    state,
};
use crate::callback_logger::CallbackLogger;
use crate::logging_success::dispatch::dispatch_success_handlers;
use crate::logging_success::payload_state::standard_logging_object;
use crate::registry::Registration;
use crate::registry::identity::CallbackIdentity;
use crate::registry::phases::CallbackList;
use crate::registry::sink::CallbackSink;

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
    let stored = standard_logging_object(&state).unwrap().unwrap();
    assert_eq!(stored.model, payload().model);
    assert_eq!(stored.messages, payload().messages);
}

fn counting_logger(
    name: &str,
    calls: &Arc<CallLog>,
    counter: &Arc<RecordingCounter>,
) -> Arc<FakeLogger> {
    let mut logger = FakeLogger::new(name, calls);
    logger.counter = Some(Arc::clone(counter));
    Arc::new(logger)
}

#[rstest]
#[case::sync_failure(false)]
#[case::async_failure(true)]
#[tokio::test]
async fn a_failed_callback_counts_once_through_the_first_counting_logger(
    #[case] asynchronous: bool,
) {
    let mut fixture = Fixture::new();
    let first_calls = Arc::new(CallLog::default());
    let first_counter = Arc::new(RecordingCounter::default());
    let mut failing = FakeLogger::new("prometheus", &first_calls);
    failing.fail = true;
    failing.counter = Some(Arc::clone(&first_counter));
    fixture.add(&Arc::new(failing));
    let second_calls = Arc::new(CallLog::default());
    let second_counter = Arc::new(RecordingCounter::default());
    fixture.add(&counting_logger(
        "datadog_metrics",
        &second_calls,
        &second_counter,
    ));
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
    assert_eq!(first_counter.calls(), ["prometheus"]);
    assert!(second_counter.calls().is_empty());
    assert_eq!(second_calls.events().len(), 1);
}

#[tokio::test]
async fn a_counter_in_the_success_list_is_found_after_the_callbacks_list() {
    let mut fixture = Fixture::new();
    let failing_calls = Arc::new(CallLog::default());
    let mut failing = FakeLogger::new("langfuse", &failing_calls);
    failing.fail = true;
    fixture.add(&Arc::new(failing));
    let counting_calls = Arc::new(CallLog::default());
    let counter = Arc::new(RecordingCounter::default());
    let counting = counting_logger("prometheus", &counting_calls, &counter);
    let identity = CallbackIdentity {
        class_name: "prometheus".to_owned(),
        attributes: IndexMap::new(),
    };
    let sink = CallbackSink::Logger(counting);
    let registration = fixture.register(identity, sink, CallbackList::Success);
    assert_ne!(registration, Registration::SkippedCapacity);
    let mut state = state(Some(false));
    state
        .litellm_params
        .insert("acompletion".to_owned(), Json::Bool(true));
    let mut correlation = correlation();
    dispatch_success_handlers(
        &fixture.handler(),
        &mut state,
        &mut correlation,
        &request(chat_result()),
    )
    .await;
    assert_eq!(counter.calls(), ["langfuse"]);
}

#[rstest]
#[case::sync_failure(false)]
#[case::async_failure(true)]
#[tokio::test]
async fn a_registry_without_a_counting_logger_counts_nothing(#[case] asynchronous: bool) {
    let mut fixture = Fixture::new();
    let calls = Arc::new(CallLog::default());
    let mut failing = FakeLogger::new("langfuse", &calls);
    failing.fail = true;
    fixture.add(&Arc::new(failing));
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
    assert_eq!(calls.events().len(), 1);
    let stored = standard_logging_object(&state).unwrap().unwrap();
    assert_eq!(stored.model, payload().model);
}

struct PlainLogger;

impl CallbackLogger for PlainLogger {
    fn callback_name(&self) -> &'static str {
        "plain"
    }
}

#[test]
fn a_logger_without_a_counter_answers_none() {
    assert!(PlainLogger.failure_counter().is_none());
}
