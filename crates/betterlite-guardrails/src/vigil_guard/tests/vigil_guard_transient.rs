use betterlite_guardrails::vigil_guard::transient::TransportError;
use reqwest::StatusCode;
use rstest::rstest;

#[rstest]
#[case::too_many_requests(StatusCode::TOO_MANY_REQUESTS)]
#[case::bad_gateway(StatusCode::BAD_GATEWAY)]
#[case::service_unavailable(StatusCode::SERVICE_UNAVAILABLE)]
#[case::gateway_timeout(StatusCode::GATEWAY_TIMEOUT)]
fn transient_status_codes_are_retried(#[case] status: StatusCode) {
    let error = TransportError::HttpStatus { status };
    assert!(error.is_transient());
}

#[rstest]
#[case::bad_request(StatusCode::BAD_REQUEST)]
#[case::unauthorized(StatusCode::UNAUTHORIZED)]
#[case::internal_server_error(StatusCode::INTERNAL_SERVER_ERROR)]
fn other_status_codes_are_not_retried(#[case] status: StatusCode) {
    let error = TransportError::HttpStatus { status };
    assert!(!error.is_transient());
}

#[rstest]
#[case::connect(TransportError::Connect)]
#[case::connect_timeout(TransportError::ConnectTimeout)]
#[case::read_timeout(TransportError::ReadTimeout)]
#[case::remote_protocol(TransportError::RemoteProtocol)]
#[case::litellm_timeout(TransportError::Timeout)]
fn connection_and_timeout_errors_are_retried(#[case] error: TransportError) {
    assert!(error.is_transient());
}

#[test]
fn other_errors_are_not_retried() {
    assert!(!TransportError::Other.is_transient());
}
