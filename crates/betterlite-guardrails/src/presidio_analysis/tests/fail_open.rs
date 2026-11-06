#![cfg(test)]

use std::time::Duration;

use axum::http::StatusCode;
use rstest::rstest;
use tracing::subscriber::set_default;

use super::shared::{Captured, NO_OVERRIDES, analyzer, json_reply, settings, spawn_mock};
use crate::presidio_analysis::wire::AnalyzeOutcome;

#[rstest]
#[case::error_dict(StatusCode::OK, "application/json", r#"{"error":"boom"}"#, "error: ")]
#[case::unexpected_type(
    StatusCode::OK,
    "application/json",
    "\"text\"",
    "unexpected type string"
)]
#[case::unparsed_dict(
    StatusCode::OK,
    "application/json",
    r#"{"entity_type":5}"#,
    "failed to parse dict response"
)]
#[case::blank_body(StatusCode::OK, "application/json", "", "unexpected type null")]
#[case::http_error(
    StatusCode::INTERNAL_SERVER_ERROR,
    "text/plain",
    "down",
    "HTTP 500 from Presidio analyzer: down"
)]
#[case::non_json(
    StatusCode::OK,
    "text/html",
    "<html>",
    "expected application/json Content-Type but received 'text/html'"
)]
#[tokio::test]
async fn fail_open_returns_an_empty_list_and_logs_a_warning(
    #[case] status: StatusCode,
    #[case] content_type: &'static str,
    #[case] body: &'static str,
    #[case] logged_reason: &str,
) {
    let captured = Captured::default();
    let writer = captured.clone();
    let _guard = set_default(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(move || writer.clone())
            .finish(),
    );
    let mock = spawn_mock(
        move |_| (status, content_type, body.to_owned()),
        Duration::ZERO,
    )
    .await;
    let outcome = analyzer(settings(&mock.base))
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap();
    assert!(matches!(outcome, AnalyzeOutcome::Items(found) if found.is_empty()));
    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("WARN"), "{log}");
    assert!(log.contains("Presidio analyzer response invalid"), "{log}");
    assert!(log.contains(logged_reason), "{log}");
}

#[tokio::test]
async fn fail_open_still_returns_valid_items() {
    let mock = spawn_mock(
        |_| json_reply(r#"[{"entity_type":"PERSON","start":0,"end":4,"score":0.9}]"#),
        Duration::ZERO,
    )
    .await;
    let AnalyzeOutcome::Items(items) = analyzer(settings(&mock.base))
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap()
    else {
        panic!("expected analyzer items");
    };
    assert_eq!(items.len(), 1);
}
