#![cfg(test)]

use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::super::ApiEventType;
use super::super::error::ApiError;
use super::super::tests::{Fixture, payload, settings};
use crate::generic_api_logger::generic_settings::GenericApiSettings;
use crate::generic_api_logger::tests::{args, capture_logs};

fn formatted(format: &str) -> GenericApiSettings {
    let mut arguments = args();
    arguments.log_format = Some(format.to_owned());
    settings(arguments)
}

async fn record_both(fixture: &Fixture) {
    let payload = payload();
    fixture
        .logger
        .record(ApiEventType::Success, Some(&payload))
        .await;
    fixture
        .logger
        .record(ApiEventType::Failure, Some(&payload))
        .await;
}

#[tokio::test]
async fn delivery_json_array_posts_one_body_with_the_resolved_headers_and_timeout() {
    let mut arguments = args();
    arguments.headers = Some(IndexMap::from([("X-Test".to_owned(), "yes".to_owned())]));
    arguments.timeout = Some(Duration::from_secs(7));
    let fixture = Fixture::new(settings(arguments), false, 10);
    record_both(&fixture).await;
    fixture.send().await;
    assert_eq!(fixture.recorder.bodies().len(), 1);
    assert_eq!(fixture.entries().len(), 2);
    let request = fixture.recorder.first();
    assert_eq!(
        request.headers.get("Content-Type"),
        Some(&"application/json".to_owned())
    );
    assert_eq!(request.headers.get("X-Test"), Some(&"yes".to_owned()));
    assert_eq!(request.timeout, Duration::from_secs(7));
}

#[tokio::test]
async fn delivery_ndjson_joins_the_entries_with_newlines() {
    let fixture = Fixture::new(formatted("ndjson"), false, 10);
    record_both(&fixture).await;
    fixture.send().await;
    let body = fixture.recorder.first().body;
    let lines: Vec<&str> = body.split('\n').collect();
    assert_eq!(lines.len(), 2);
    assert!(
        lines
            .iter()
            .all(|line| serde_json::from_str::<Json>(line).is_ok())
    );
    assert!(!body.ends_with('\n'));
}

#[tokio::test]
async fn delivery_single_posts_each_entry_and_continues_past_a_failure() {
    let fixture = Fixture::new(formatted("single"), false, 10);
    fixture.recorder.reply(Err(ApiError::Timeout));
    let (logs, _guard) = capture_logs();
    record_both(&fixture).await;
    fixture.send().await;
    assert_eq!(fixture.recorder.bodies().len(), 2);
    assert!(
        logs.contents()
            .contains("Generic API Logger - Error sending log 0: the request timed out")
    );
}

#[tokio::test]
async fn delivery_clears_the_queue_after_a_failed_send() {
    let fixture = Fixture::new(settings(args()), false, 10);
    fixture.recorder.reply(Err(ApiError::Timeout));
    record_both(&fixture).await;
    fixture.send().await;
    fixture.send().await;
    assert_eq!(fixture.recorder.bodies().len(), 1);
}
