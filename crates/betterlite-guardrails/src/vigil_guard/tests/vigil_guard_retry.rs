use betterlite_guardrails::vigil_guard::DEFAULT_TIMEOUT;
use betterlite_guardrails::vigil_guard::error::VigilGuardError;
use betterlite_guardrails::vigil_guard::handler_protocol::HttpResponse;
use betterlite_guardrails::vigil_guard::retry::post_with_retry;
use betterlite_guardrails::vigil_guard::transient::TransportError;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use rstest::rstest;

use crate::support::{FakeHandler, json_body, payload, status_only};

const ENDPOINT: &str = "https://vigil.example/v1/guard/analyze";

#[rstest]
#[case::too_many_requests(StatusCode::TOO_MANY_REQUESTS)]
#[case::bad_gateway(StatusCode::BAD_GATEWAY)]
#[case::service_unavailable(StatusCode::SERVICE_UNAVAILABLE)]
#[case::gateway_timeout(StatusCode::GATEWAY_TIMEOUT)]
#[tokio::test]
async fn retries_once_on_transient_status(#[case] status: StatusCode) {
    let handler = FakeHandler::new(vec![Ok(status_only(status)), Ok(json_body("{}"))]);
    let headers = HeaderMap::new();
    let response = post_with_retry(&handler, ENDPOINT, &headers, &payload(), DEFAULT_TIMEOUT)
        .await
        .expect("the second attempt succeeds");
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(handler.calls().len(), 2);
}

#[rstest]
#[case::connect(TransportError::Connect)]
#[case::connect_timeout(TransportError::ConnectTimeout)]
#[case::read_timeout(TransportError::ReadTimeout)]
#[case::remote_protocol(TransportError::RemoteProtocol)]
#[case::litellm_timeout(TransportError::Timeout)]
#[tokio::test]
async fn retries_once_on_transient_transport_error(#[case] error: TransportError) {
    let handler = FakeHandler::new(vec![Err(error), Ok(json_body("{}"))]);
    let headers = HeaderMap::new();
    let response = post_with_retry(&handler, ENDPOINT, &headers, &payload(), DEFAULT_TIMEOUT)
        .await
        .expect("the second attempt succeeds");
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(handler.calls().len(), 2);
}

#[tokio::test]
async fn does_not_retry_a_client_error() {
    let handler = FakeHandler::new(vec![Ok(status_only(StatusCode::BAD_REQUEST))]);
    let headers = HeaderMap::new();
    let error = post_with_retry(&handler, ENDPOINT, &headers, &payload(), DEFAULT_TIMEOUT)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        VigilGuardError::Transport {
            source: TransportError::HttpStatus { status }
        } if status == StatusCode::BAD_REQUEST
    ));
    assert_eq!(handler.calls().len(), 1);
}

#[tokio::test]
async fn fails_after_the_second_transient_failure() {
    let handler = FakeHandler::new(vec![
        Ok(status_only(StatusCode::SERVICE_UNAVAILABLE)),
        Ok(status_only(StatusCode::SERVICE_UNAVAILABLE)),
    ]);
    let headers = HeaderMap::new();
    let error = post_with_retry(&handler, ENDPOINT, &headers, &payload(), DEFAULT_TIMEOUT)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        VigilGuardError::Transport {
            source: TransportError::HttpStatus { status }
        } if status == StatusCode::SERVICE_UNAVAILABLE
    ));
    assert_eq!(handler.calls().len(), 2);
}

#[tokio::test]
async fn does_not_retry_other_transport_errors() {
    let handler = FakeHandler::new(vec![Err(TransportError::Other)]);
    let headers = HeaderMap::new();
    let error = post_with_retry(&handler, ENDPOINT, &headers, &payload(), DEFAULT_TIMEOUT)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        VigilGuardError::Transport {
            source: TransportError::Other
        }
    ));
    assert_eq!(handler.calls().len(), 1);
}

#[tokio::test]
async fn second_attempt_is_not_retried() {
    let handler = FakeHandler::new(vec![
        Ok(status_only(StatusCode::BAD_GATEWAY)),
        Ok(status_only(StatusCode::BAD_REQUEST)),
    ]);
    let headers = HeaderMap::new();
    let error = post_with_retry(&handler, ENDPOINT, &headers, &payload(), DEFAULT_TIMEOUT)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        VigilGuardError::Transport {
            source: TransportError::HttpStatus { status }
        } if status == StatusCode::BAD_REQUEST
    ));
    assert_eq!(handler.calls().len(), 2);
}

#[rstest]
#[case::continue_status(StatusCode::CONTINUE)]
#[case::moved_permanently(StatusCode::MOVED_PERMANENTLY)]
#[case::not_modified(StatusCode::NOT_MODIFIED)]
#[tokio::test]
async fn non_success_status_with_a_json_body_is_a_failure(#[case] status: StatusCode) {
    let response = HttpResponse {
        status,
        body: br#"{"decision":"ALLOWED"}"#.to_vec(),
    };
    let handler = FakeHandler::new(vec![Ok(response)]);
    let headers = HeaderMap::new();
    let error = post_with_retry(&handler, ENDPOINT, &headers, &payload(), DEFAULT_TIMEOUT)
        .await
        .unwrap_err();
    let VigilGuardError::Transport { source } = error else {
        panic!("expected a transport error, got {error:?}");
    };
    assert_eq!(source, TransportError::HttpStatus { status });
    assert!(!source.is_transient());
    assert_eq!(handler.calls().len(), 1);
}

#[tokio::test]
async fn success_returns_without_a_retry() {
    let handler = FakeHandler::new(vec![Ok(json_body("{}"))]);
    let headers = HeaderMap::new();
    post_with_retry(&handler, ENDPOINT, &headers, &payload(), DEFAULT_TIMEOUT)
        .await
        .expect("the first attempt succeeds");
    assert_eq!(handler.calls().len(), 1);
}
