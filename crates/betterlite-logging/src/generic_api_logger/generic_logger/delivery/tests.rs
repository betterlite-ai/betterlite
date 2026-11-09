#![cfg(test)]
use super::super::ApiEventType;
use super::super::GenericApiLogger;
use super::super::error::ApiError;
use super::super::tests::{entries, payload, ready, settings};
use crate::generic_api_logger::tests::{args, capture_logs};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use std::time::Duration;
use tokio::time::timeout;
async fn record_both(logger: &GenericApiLogger) {
    let payload = payload();
    logger.record(ApiEventType::Success, &payload).await;
    logger.record(ApiEventType::Failure, &payload).await;
}
#[tokio::test]
async fn delivery_posts_the_queue_in_the_selected_format() {
    let mut arguments = args();
    arguments.headers = Some(IndexMap::from([("X-Test".to_owned(), "yes".to_owned())]));
    arguments.timeout = Some(Duration::from_secs(7));
    let (recorder, _sleeper, logger) = ready(settings(arguments), false, 10);
    record_both(&logger).await;
    logger.send_batch().await;
    assert_eq!(recorder.bodies().len(), 1);
    assert_eq!(entries(&recorder).len(), 2);
    let request = recorder.first();
    assert_eq!(
        request.headers.get("Content-Type"),
        Some(&"application/json".to_owned())
    );
    assert_eq!(request.headers.get("X-Test"), Some(&"yes".to_owned()));
    assert_eq!(request.timeout, Duration::from_secs(7));
    let mut ndjson_arguments = args();
    ndjson_arguments.log_format = Some("ndjson".to_owned());
    let (ndjson, _ndjson_sleeper, ndjson_logger) = ready(settings(ndjson_arguments), false, 10);
    record_both(&ndjson_logger).await;
    ndjson_logger.send_batch().await;
    let body = ndjson.first().body;
    let lines: Vec<&str> = body.split('\n').collect();
    assert_eq!(lines.len(), 2);
    assert!(!body.ends_with('\n'));
    assert!(
        lines
            .iter()
            .all(|line| serde_json::from_str::<Json>(line).is_ok())
    );
}
#[tokio::test]
async fn delivery_single_posts_each_entry_and_continues_past_a_failure() {
    let mut arguments = args();
    arguments.log_format = Some("single".to_owned());
    let (recorder, _sleeper, logger) = ready(settings(arguments), false, 10);
    recorder.reply(Err(ApiError::Timeout));
    let (logs, _guard) = capture_logs();
    record_both(&logger).await;
    logger.send_batch().await;
    assert_eq!(recorder.bodies().len(), 2);
    assert!(
        logs.contents()
            .contains("Generic API Logger - Error sending log 0: the request timed out")
    );
}
#[tokio::test]
async fn delivery_clears_the_queue_after_a_failed_or_cancelled_send() {
    let (recorder, _sleeper, logger) = ready(settings(args()), false, 10);
    recorder.reply(Err(ApiError::Timeout));
    record_both(&logger).await;
    logger.send_batch().await;
    logger.send_batch().await;
    assert_eq!(recorder.bodies().len(), 1);
    let (cancelled, _cancel_sleeper, cancelled_logger) = ready(settings(args()), false, 10);
    cancelled.block();
    record_both(&cancelled_logger).await;
    let dropped = timeout(Duration::from_millis(5), cancelled_logger.send_batch()).await;
    assert!(dropped.is_err());
    cancelled_logger.send_batch().await;
    assert_eq!(cancelled.bodies().len(), 1);
}
#[tokio::test]
async fn delivery_keeps_an_event_recorded_while_a_post_is_in_flight() {
    let (recorder, _sleeper, logger) = ready(settings(args()), false, 10);
    let held = payload();
    logger.record(ApiEventType::Success, &held).await;
    recorder.block();
    let started = recorder.started.notified();
    let recorded_later = async {
        started.await;
        let later = payload();
        logger.record(ApiEventType::Success, &later).await;
        recorder.gate.notify_one();
    };
    tokio::join!(logger.send_batch(), recorded_later);
    logger.send_batch().await;
    let bodies = recorder.bodies();
    assert_eq!(bodies.len(), 2);
    assert!(
        bodies
            .iter()
            .all(|body| matches!(body, Json::List(entries) if entries.len() == 1))
    );
}
