use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::callback_logger::CallbackLogger as _;
use crate::exporter_options::redaction::ResponseStamp;
use crate::zerobus::logger::ZerobusLogger;
use crate::zerobus::logger::delivery::RowIngest;
use crate::zerobus::logger::payload::{EventDetails, PayloadPolicy, payload_for};

use super::support::{
    Buffer, FakeIngest, deps, event, event_with, logger, params, payload, queued_ids, queued_rows,
    subscribe,
};

fn dynamic_flag(value: bool) -> Json {
    Json::Object(IndexMap::from([(
        "turn_off_message_logging".to_owned(),
        Json::Bool(value),
    )]))
}

fn assert_redacted(logger: &ZerobusLogger) {
    let rows = queued_rows(logger);
    assert_eq!(rows.len(), 1);
    let messages = rows[0].messages.clone().expect("messages are replaced");
    assert!(messages.contains("redacted-by-litellm"), "{messages}");
    assert!(!messages.contains("hi"), "{messages}");
    let response = rows[0].response.clone().expect("the response is replaced");
    assert!(!response.contains("the secret answer"), "{response}");
}

#[tokio::test]
async fn zerobus_privacy_keeps_the_payload_without_a_flag() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let logger = logger(params(10, 3600, false), client);
    logger
        .async_log_success_event(event("a"))
        .await
        .expect("the event is accepted");
    let rows = queued_rows(&logger);
    assert_eq!(rows.len(), 1);
    let messages = rows[0].messages.clone().expect("messages are kept");
    assert!(messages.contains("hi"), "{messages}");
    let response = rows[0].response.clone().expect("the response is kept");
    assert!(response.contains("the secret answer"), "{response}");
}

#[tokio::test]
async fn zerobus_privacy_redacts_for_a_dynamic_flag() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let logger = logger(params(10, 3600, false), client);
    logger
        .async_log_success_event(event_with(
            payload("a"),
            dynamic_flag(true),
            IndexMap::new(),
        ))
        .await
        .expect("the event is accepted");
    assert_redacted(&logger);
    let rows = queued_rows(&logger);
    assert_eq!(rows[0].model.as_deref(), Some("gpt-4o"));
}

#[tokio::test]
async fn zerobus_privacy_redacts_for_the_global_setting() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let mut configured = deps(params(10, 3600, false), Some(client));
    configured.redaction.turn_off_message_logging = true;
    let logger = ZerobusLogger::new(configured).expect("the logger builds");
    logger
        .async_log_success_event(event("a"))
        .await
        .expect("the event is accepted");
    assert_redacted(&logger);
}

#[tokio::test]
async fn zerobus_privacy_redacts_for_the_logger_setting() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let logger = logger(params(10, 3600, true), client);
    logger
        .async_log_success_event(event("a"))
        .await
        .expect("the event is accepted");
    assert_redacted(&logger);
    let rows = queued_rows(&logger);
    let response = rows[0].response.clone().expect("the response is replaced");
    assert!(response.contains("1704067201"), "{response}");
}

#[tokio::test]
async fn zerobus_privacy_keeps_the_payload_when_the_disable_header_is_set() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let logger = logger(params(10, 3600, false), client);
    let metadata = IndexMap::from([(
        "headers".to_owned(),
        Json::Object(IndexMap::from([(
            "litellm-disable-message-redaction".to_owned(),
            Json::Bool(true),
        )])),
    )]);
    logger
        .async_log_success_event(event_with(payload("a"), dynamic_flag(true), metadata))
        .await
        .expect("the event is accepted");
    let rows = queued_rows(&logger);
    let messages = rows[0].messages.clone().expect("messages are kept");
    assert!(messages.contains("hi"), "{messages}");
}

#[tokio::test]
async fn zerobus_privacy_reads_a_dynamic_false_over_the_global_setting() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let mut configured = deps(params(10, 3600, false), Some(client));
    configured.redaction.turn_off_message_logging = true;
    let logger = ZerobusLogger::new(configured).expect("the logger builds");
    logger
        .async_log_success_event(event_with(
            payload("a"),
            dynamic_flag(false),
            IndexMap::new(),
        ))
        .await
        .expect("the event is accepted");
    let rows = queued_rows(&logger);
    let messages = rows[0].messages.clone().expect("messages are kept");
    assert!(messages.contains("hi"), "{messages}");
}

#[tokio::test]
async fn zerobus_privacy_removes_an_excluded_field() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let mut configured = deps(params(10, 3600, false), Some(client));
    configured.redaction.excluded_fields = vec!["end_user".to_owned()];
    let logger = ZerobusLogger::new(configured).expect("the logger builds");
    logger
        .async_log_success_event(event("a"))
        .await
        .expect("the event is accepted");
    let rows = queued_rows(&logger);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].end_user.is_none());
    assert_eq!(rows[0].model.as_deref(), Some("gpt-4o"));
    let messages = rows[0].messages.clone().expect("messages are kept");
    assert!(messages.contains("hi"), "{messages}");
}

#[tokio::test]
async fn zerobus_privacy_drops_a_payload_that_cannot_be_read_back() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let mut configured = deps(params(10, 3600, false), Some(client));
    configured.redaction.excluded_fields = vec!["model".to_owned()];
    let logger = ZerobusLogger::new(configured).expect("the logger builds");
    let buffer = Buffer::default();
    let guard = subscribe(&buffer);
    logger
        .async_log_success_event(event_with(
            payload("a"),
            dynamic_flag(true),
            IndexMap::new(),
        ))
        .await
        .expect("the event is accepted");
    drop(guard);
    assert!(queued_ids(&logger).is_empty());
    assert!(
        buffer
            .contents()
            .contains("dropped instead of logged unredacted"),
        "{}",
        buffer.contents()
    );
}

#[test]
fn zerobus_privacy_skips_a_missing_logging_object() {
    let metadata: IndexMap<String, Json> = IndexMap::new();
    let dynamic = Json::Null;
    let details = EventDetails {
        standard_logging_object: None,
        stamp: ResponseStamp {
            id: "a".to_owned(),
            created: 0,
        },
        litellm_metadata: &metadata,
        dynamic_params: &dynamic,
    };
    let policy = PayloadPolicy {
        turn_off_message_logging: false,
        excluded_fields: &[],
        global_turn_off: false,
    };
    assert!(payload_for(&details, &policy).is_none());
}
