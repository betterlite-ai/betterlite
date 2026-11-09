#![cfg(test)]

use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use rstest::rstest;

use super::super::error::ApiError;
use super::super::tests::{Fixture, settings};
use super::post_with_retries;
use crate::generic_api_logger::tests::{ENDPOINT, args, capture_logs};

async fn run(
    max_retries: i64,
    retry_delay: Option<f64>,
    replies: &[Result<u16, ApiError>],
) -> (Result<(), ApiError>, Fixture) {
    let mut arguments = args();
    arguments.max_retries = Some(max_retries);
    arguments.retry_delay = retry_delay;
    let fixture = Fixture::new(settings(arguments), false, 10);
    for reply in replies {
        let outcome = reply
            .as_ref()
            .map_or_else(|_| Err(ApiError::Timeout), |status| Ok(*status));
        fixture.recorder.reply(outcome);
    }
    let result = post_with_retries(&fixture.logger, "{}").await;
    (result, fixture)
}

#[tokio::test]
async fn retry_makes_max_retries_plus_one_attempts_and_retries_a_server_error() {
    let (result, fixture) = run(1, None, &[Ok(503), Ok(200)]).await;
    result.expect("the retry succeeds");
    assert_eq!(fixture.recorder.bodies().len(), 2);
    let (failed, attempts) = run(
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
    assert_eq!(attempts.recorder.bodies().len(), 3);
}

#[tokio::test]
async fn retry_does_not_retry_a_client_error() {
    let (result, fixture) = run(3, None, &[Ok(404)]).await;
    assert!(matches!(result, Err(ApiError::Status { status: 404 })));
    assert_eq!(fixture.recorder.bodies().len(), 1);
}

#[tokio::test]
async fn retry_sleeps_a_doubling_delay_and_skips_a_zero_delay() {
    let (result, fixture) = run(
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
    assert_eq!(
        fixture.waits(),
        [Duration::from_millis(500), Duration::from_secs(1)]
    );
    let (retried, second) = run(1, None, &[Err(ApiError::Timeout), Ok(200)]).await;
    retried.expect("the retry succeeds");
    assert!(second.waits().is_empty());
}

#[tokio::test]
async fn retry_logs_each_retry_with_the_endpoint_and_attempt() {
    let (logs, _guard) = capture_logs();
    let (result, _fixture) = run(1, None, &[Err(ApiError::Timeout), Ok(200)]).await;
    result.expect("the retry succeeds");
    let expected = format!(
        "Generic API Logger - retrying request to {ENDPOINT} after error: the request timed out (attempt 1/2)"
    );
    assert!(logs.contents().contains(&expected));
}

#[rstest]
#[case::timeout(ApiError::Timeout, true)]
#[case::server_error(ApiError::Status { status: 500 }, true)]
#[case::server_unavailable(ApiError::Status { status: 503 }, true)]
#[case::client_error(ApiError::Status { status: 404 }, false)]
#[case::rate_limited(ApiError::Status { status: 429 }, false)]
#[case::transport(ApiError::Transport(TransportError::TooManyRedirects { limit: 5 }), true)]
#[case::closed(ApiError::Transport(TransportError::Closed), false)]
fn retry_classifies_an_error(#[case] error: ApiError, #[case] retryable: bool) {
    assert_eq!(error.is_retryable(), retryable);
}
