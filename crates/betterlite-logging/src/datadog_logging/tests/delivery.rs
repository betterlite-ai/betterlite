use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};

use crate::callback_logger::CallbackLogger as _;
use crate::datadog_logging::payloads::{DatadogLoggingKwargs, SyncCall};
use crate::datadog_logging::sync;
use crate::datadog_logging::tests::support;
use crate::fields::payload::StandardLoggingPayload;

mod datadog_delivery_sync_async_behavior;

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").expect("a valid timestamp")
}

fn kwargs(payload: StandardLoggingPayload) -> DatadogLoggingKwargs {
    DatadogLoggingKwargs {
        standard_logging_object: Some(payload),
        legacy: None,
    }
}

fn legacy_kwargs() -> DatadogLoggingKwargs {
    DatadogLoggingKwargs {
        standard_logging_object: None,
        legacy: Some(
            serde_json::from_str(r#"{"model": "gpt-4o", "response_cost": 0.25}"#)
                .expect("legacy kwargs"),
        ),
    }
}

fn field(body: &Json, name: &str) -> Json {
    let Json::Object(fields) = body else {
        panic!("a payload body is an object");
    };
    fields.get(name).cloned().expect("the field is present")
}

fn message_field(body: &Json, name: &str) -> Json {
    let Json::Text(message) = field(body, "message") else {
        panic!("the envelope message is a JSON string");
    };
    let inner: Json = serde_json::from_str(&message).expect("the message is JSON");
    field(&inner, name)
}

fn status_of(body: &Json) -> String {
    match field(body, "status") {
        Json::Text(status) => status,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("a payload carries its status")
        }
    }
}

#[tokio::test]
async fn datadog_delivery_sync_async_behavior() {
    let (logger, transport) = support::build(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &support::Environment::new(),
        vec![support::Reply::accepted()],
    );
    let payload = kwargs(support::standard_payload("sync-1", false));
    let call = SyncCall {
        kwargs: &payload,
        response_obj: &Json::Null,
        start_time: moment(),
        end_time: moment(),
    };
    sync::log_success_event(&logger, &call).await;
    assert_eq!(transport.call_count(), 1);
    assert_eq!(logger.queue.pending().len(), 0);
    let sent = transport.calls();
    assert_eq!(sent[0].header("content-encoding"), None);
    assert_eq!(status_of(&sent[0].body), "info");

    let (async_logger, async_transport) = support::build(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &support::Environment::new(),
        Vec::new(),
    );
    async_logger
        .async_log_success_event(support::event(support::standard_payload("async-1", false)))
        .await
        .expect("the async event is accepted");
    assert_eq!(async_transport.call_count(), 0);
    assert_eq!(support::message_ids(&async_logger), vec!["async-1"]);
}

#[tokio::test]
async fn datadog_delivery_async_failure_carries_the_error_status() {
    let (logger, transport) = support::build(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &support::Environment::new(),
        Vec::new(),
    );
    logger
        .async_log_failure_event(support::event(support::standard_payload("fail-1", true)))
        .await
        .expect("the async event is accepted");
    assert_eq!(transport.call_count(), 0);
    let queued = support::queued(&logger);
    assert_eq!(status_of(&queued[0]), "error");
}

#[tokio::test]
async fn datadog_delivery_sync_extra_status_is_an_error() {
    let (_guard, logs) = support::capture_logs();
    let (logger, transport) = support::build(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &support::Environment::new(),
        vec![support::Reply::status(200)],
    );
    let payload = kwargs(support::standard_payload("sync-1", false));
    let call = SyncCall {
        kwargs: &payload,
        response_obj: &Json::Null,
        start_time: moment(),
        end_time: moment(),
    };
    sync::log_success_event(&logger, &call).await;
    assert_eq!(transport.call_count(), 1);
    assert_eq!(logger.queue.pending().len(), 0);
    assert_eq!(logs.count_containing("Datadog Layer Error"), 1);
}

#[tokio::test]
async fn datadog_delivery_sync_without_a_standard_payload_sends_nothing() {
    let (logger, transport) = support::build(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &support::Environment::new(),
        vec![support::Reply::accepted()],
    );
    let empty = DatadogLoggingKwargs {
        standard_logging_object: None,
        legacy: None,
    };
    let call = SyncCall {
        kwargs: &empty,
        response_obj: &Json::Null,
        start_time: moment(),
        end_time: moment(),
    };
    sync::log_success_event(&logger, &call).await;
    assert_eq!(transport.call_count(), 0);
}

#[tokio::test]
async fn datadog_delivery_flushes_at_the_batch_size() {
    let environment = support::environment(&[("DD_BATCH_SIZE", "2")]);
    let (logger, transport) = support::build(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &environment,
        vec![support::Reply::accepted()],
    );
    logger
        .async_log_success_event(support::event(support::standard_payload("one", false)))
        .await
        .expect("the async event is accepted");
    assert_eq!(transport.call_count(), 0);
    assert_eq!(logger.queue.pending().len(), 1);
    logger
        .async_log_success_event(support::event(support::standard_payload("two", false)))
        .await
        .expect("the async event is accepted");
    assert_eq!(transport.call_count(), 1);
    assert_eq!(logger.queue.pending().len(), 0);
    assert_eq!(transport.calls()[0].entries().len(), 2);
}
