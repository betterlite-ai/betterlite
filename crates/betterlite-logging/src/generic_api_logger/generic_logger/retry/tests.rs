#![cfg(test)]
use super::super::error::ApiError;
use super::super::tests::{RecordingSleeper, RecordingTransport, ready, settings};
use super::post_with_retries;
use crate::generic_api_logger::tests::{ENDPOINT, args, capture_logs};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{
    ProxiedClient,
    outbound::{OutboundRequest, RequestBody},
};
use http::{Method, header::HeaderMap};
use rstest::rstest;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use url::Url;
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
async fn retry_uses_max_retries_plus_one_attempts_and_only_retries_retryable_errors() {
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
    let (client_error, single, _single_sleeper) = run(2, None, &[Ok(404)]).await;
    assert!(matches!(
        client_error,
        Err(ApiError::Status { status: 404 })
    ));
    assert_eq!(single.bodies().len(), 1);
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
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("a policy");
    let client = ProxiedClient::outbound(&Arc::new(policy)).expect("a client");
    let request = OutboundRequest {
        method: Method::POST,
        url: Url::parse("http://127.0.0.1:1/").expect("a URL"),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_secs(5),
    };
    let failure = client
        .send_buffered(request)
        .await
        .expect_err("the port is closed");
    assert!(matches!(failure, TransportError::Connect(_)));
    assert!(ApiError::Transport(failure).is_retryable());
}
#[rstest]
#[case::timeout(ApiError::Timeout, true)]
#[case::server_error(ApiError::Status { status: 500 }, true)]
#[case::client_error(ApiError::Status { status: 404 }, false)]
#[case::redirects(ApiError::Transport(TransportError::TooManyRedirects { limit: 5 }), false)]
fn retry_classifies_an_error(#[case] error: ApiError, #[case] retryable: bool) {
    assert_eq!(error.is_retryable(), retryable);
}
