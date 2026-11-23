use std::sync::Arc;

use betterlite_models::decisions::Json;
use tokio::sync::oneshot::{self, Sender};

use super::support::{self, Answer, AnswerStep, POLLING_ID, RecordingCache, Scenario};
use crate::api::responses_poll_state::access::PollAccess;
use crate::api::responses_poll_stream::worker::PollStreamError;

const ITEM_ADDED: &str =
    r#"{"item":{"content":[],"id":"m1","type":"message"},"type":"response.output_item.added"}"#;
const DELTA: &str =
    r#"{"content_index":0,"delta":"A","item_id":"m1","type":"response.output_text.delta"}"#;
const COMPLETED: &str =
    r#"{"response":{"output":[],"status":"completed"},"type":"response.completed"}"#;

pub(super) async fn cancelled_mid_stream() -> Scenario {
    let mut steps: Vec<AnswerStep> = support::frames(&[ITEM_ADDED, DELTA])
        .into_iter()
        .map(AnswerStep::Chunk)
        .collect();
    let (sender, receiver) = oneshot::channel();
    steps.push(AnswerStep::Wait(receiver));
    steps.push(AnswerStep::Chunk(support::frames(&[COMPLETED]).remove(0)));
    let mut scenario = support::scenario(&[1000], Answer::Streaming(steps));
    let cache = Arc::clone(&scenario.cache);
    let (outcome, ()) = tokio::join!(scenario.run(), cancel(&cache, sender));
    outcome.expect("the worker completes after the cancellation");
    scenario
}

async fn cancel(cache: &Arc<RecordingCache>, release: Sender<()>) {
    let store = support::store_for(cache);
    store
        .cancel_polling(POLLING_ID, PollAccess::AuthDisabled)
        .await
        .expect("the cancellation writes");
    release.send(()).expect("the worker waits on the release");
}

pub(super) async fn failed_processor() -> (Scenario, Vec<String>) {
    let (guard, logs) = support::capture_logs();
    let mut scenario = support::scenario(&[1000], Answer::Failure(String::from("boom")));
    scenario.run().await.expect("the failure is recorded");
    drop(guard);
    (scenario, logs.messages("ERROR"))
}

pub(super) async fn long_failed_processor(message: String) -> Scenario {
    let mut scenario = support::scenario(&[1000], Answer::Failure(message));
    scenario.run().await.expect("the failure is recorded");
    scenario
}

pub(super) async fn failed_chunk_source(message: &str) -> Scenario {
    let mut steps: Vec<AnswerStep> = support::frames(&[ITEM_ADDED])
        .into_iter()
        .map(AnswerStep::Chunk)
        .collect();
    steps.push(AnswerStep::Fail(String::from(message)));
    let mut scenario = support::scenario(&[1000], Answer::Streaming(steps));
    scenario.run().await.expect("the failure is recorded");
    scenario
}

pub(super) fn assert_failed_message(scenario: &Scenario, message: &str) {
    let record = support::last_write(&scenario.cache);
    assert_eq!(support::text(&record, "status"), "failed");
    let error = support::field(&record, "error");
    assert_eq!(support::text(&error, "message"), message);
    assert_eq!(support::text(&error, "code"), "background_streaming_error");
}

pub(super) async fn failed_write() -> (Scenario, PollStreamError) {
    let mut scenario = support::scenario(&[1000], Answer::Failure(String::from("boom")));
    scenario.cache.fail_writes_from(1);
    let error = scenario
        .run()
        .await
        .expect_err("the failed record write propagates");
    (scenario, error)
}

pub(super) fn message_of(scenario: &Scenario) -> String {
    let record = support::last_write(&scenario.cache);
    support::text(&support::field(&record, "error"), "message")
}

pub(super) fn assert_cancelled_then_completed(scenario: &Scenario) {
    let statuses: Vec<String> = scenario
        .cache
        .writes()
        .iter()
        .map(|record| support::text(record, "status"))
        .collect();
    assert!(statuses.contains(&String::from("cancelled")));
    assert_eq!(
        support::text(&support::last_write(&scenario.cache), "status"),
        "completed"
    );
}

pub(super) fn assert_failed_record(scenario: &Scenario) {
    let record = support::last_write(&scenario.cache);
    assert_eq!(support::text(&record, "status"), "failed");
    let error = support::field(&record, "error");
    assert_eq!(support::text(&error, "type"), "internal_error");
    assert_eq!(support::text(&error, "message"), "boom");
    assert_eq!(support::text(&error, "code"), "background_streaming_error");
    let Json::Object(fields) = &error else {
        panic!("the error is an object");
    };
    assert_eq!(
        fields.keys().cloned().collect::<Vec<String>>(),
        vec![
            String::from("type"),
            String::from("message"),
            String::from("code")
        ]
    );
}

pub(super) fn assert_error_logged(messages: &[String]) {
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Error in background streaming task"))
    );
}

pub(super) fn assert_write_error(scenario: &Scenario, error: PollStreamError) {
    assert!(matches!(error, PollStreamError::Polling(_)));
    assert_eq!(scenario.cache.writes().len(), 1);
}

pub(super) fn assert_redacted(message: &str) {
    assert!(message.contains("REDACTED"));
    assert!(!message.contains("sk-"));
    assert!(!message.contains("Traceback"));
    assert!(!message.contains("secret tail"));
}

pub(super) fn assert_bounded(message: &str) {
    assert_eq!(message.len(), 16_384);
}
