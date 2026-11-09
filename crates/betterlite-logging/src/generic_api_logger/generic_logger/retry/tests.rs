#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use rstest::rstest;

use super::super::error::ApiError;
use super::super::tests::{RecordingSleeper, RecordingTransport, ready, settings};
use super::post_with_retries;
use crate::generic_api_logger::tests::{ENDPOINT, args, capture_logs};
async fn run(
    max_retries: i64,
    retry_delay: Option<f64>,
    replies: &[Result<u16, ApiError>],
) -> (
    Result<(), ApiError>,
    Arc<RecordingTransport>,
    Arc<RecordingSleeper>,
) {
    let mut arguments = args();
    arguments.max_retries = Some(max_retries);
    arguments.retry_delay = retry_delay;
    let (recorder, sleeper, logger) = ready(settings(arguments), false, 10);
    for reply in replies {
        let outcome = reply
            .as_ref()
            .map_or_else(|_| Err(ApiError::Timeout), |status| Ok(*status));
        recorder.reply(outcome);
    }
    let result = post_with_retries(&logger, "{}").await;
    (result, recorder, sleeper)
}
#[tokio::test]
async fn retry_makes_max_retries_plus_one_attempts_and_retries_a_server_error() {
    let (logs, _guard) = capture_logs();
    let (result, recorder, _sleeper) = run(1, None, &[Ok(503), Ok(200)]).await;
    result.expect("the retry succeeds");
    assert_eq!(recorder.bodies().len(), 2);
    let warning = format!(
        "Generic API Logger - retrying request to {ENDPOINT} after error: the destination answered with status 503 (attempt 1/2)"
    );
    assert!(logs.contents().contains(&warning));
    let (failed, attempts, _attempts_sleeper) = run(
        2,
        None,
        &[
            Err(ApiError::Timeout),
            Err(ApiError::Timeout),
            Err(ApiError::Timeout),
        ],
    )
    .await;
    assert!(matches!(failed, Err(ApiError::Timeout)));
    assert_eq!(attempts.bodies().len(), 3);
}
#[tokio::test]
async fn retry_does_not_retry_a_client_error() {
    let (result, recorder, _sleeper) = run(2, None, &[Ok(404)]).await;
    assert!(matches!(result, Err(ApiError::Status { status: 404 })));
    assert_eq!(recorder.bodies().len(), 1);
}
#[tokio::test]
async fn retry_sleeps_a_doubling_delay_and_skips_a_zero_delay() {
    let (result, _recorder, sleeper) = run(
        2,
        Some(0.5_f64),
        &[
            Err(ApiError::Timeout),
            Err(ApiError::Timeout),
            Err(ApiError::Timeout),
        ],
    )
    .await;
    assert!(result.is_err());
    let waits = sleeper.waits.lock().expect("the waits").clone();
    assert_eq!(waits, [Duration::from_millis(500), Duration::from_secs(1)]);
    let (retried, _skipped_recorder, skipped_by) =
        run(1, None, &[Err(ApiError::Timeout), Ok(200)]).await;
    retried.expect("the retry succeeds");
    assert!(skipped_by.waits.lock().expect("the waits").is_empty());
}
#[tokio::test]
async fn retry_treats_a_connection_failure_as_retryable() {
    let error = reqwest::get("http://127.0.0.1:1/")
        .await
        .expect_err("nothing listens on port 1");
    let connection = TransportError::sending(error);
    assert!(matches!(connection, TransportError::Connect(_)));
    assert!(ApiError::Transport(connection).is_retryable());
}
#[rstest]
#[case::timeout(ApiError::Timeout, true)]
#[case::server_error(ApiError::Status { status: 500 }, true)]
#[case::client_error(ApiError::Status { status: 404 }, false)]
#[case::rate_limited(ApiError::Status { status: 429 }, false)]
#[case::redirects(ApiError::Transport(TransportError::TooManyRedirects { limit: 5 }), false)]
fn retry_classifies_an_error(#[case] error: ApiError, #[case] retryable: bool) {
    assert_eq!(error.is_retryable(), retryable);
}
