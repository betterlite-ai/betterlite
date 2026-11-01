#![cfg(test)]
use std::net::TcpListener;

use betterlite_guardrails::deepkeep::error::DeepKeepError;
use reqwest::StatusCode;
use reqwest::header::{HeaderName, HeaderValue};
use rstest::rstest;

use super::deepkeep_failure;

fn utf8_error() -> std::str::Utf8Error {
    let invalid: Vec<u8> = std::iter::repeat_n(0xFF_u8, 1).collect();
    std::str::from_utf8(&invalid).unwrap_err()
}

fn connection_refused() -> reqwest::Error {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime
        .block_on(reqwest::Client::new().get(format!("http://{addr}")).send())
        .unwrap_err()
}

#[rstest]
#[case::blocked(
    DeepKeepError::Blocked {
        reason: "policy".to_owned(),
    },
    Some(400)
)]
#[case::api(
    DeepKeepError::Api {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        body: "boom".to_owned(),
    },
    None
)]
#[case::api_encoding(
    DeepKeepError::ApiEncoding {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        source: utf8_error(),
    },
    None
)]
#[case::response_json(
    DeepKeepError::ResponseJson {
        source: serde_json::from_str::<serde_json::Value>("not json").unwrap_err(),
    },
    None
)]
#[case::response_too_large(DeepKeepError::ResponseTooLarge { limit: 10 }, None)]
#[case::request(DeepKeepError::Request { source: connection_refused() }, None)]
#[case::timeout(DeepKeepError::Timeout { source: connection_refused() }, None)]
#[case::request_build(DeepKeepError::RequestBuild { source: connection_refused() }, None)]
#[case::invalid_header(
    DeepKeepError::InvalidHeader {
        name: "deepkeep-api-key",
        source: HeaderValue::from_str("\u{0}").unwrap_err(),
    },
    None
)]
#[case::invalid_header_name(
    DeepKeepError::InvalidHeaderName {
        name: "\u{0}".to_owned(),
        source: HeaderName::from_bytes(b"\0").unwrap_err(),
    },
    None
)]
fn deepkeep_failures_carry_the_python_status(
    #[case] error: DeepKeepError,
    #[case] status: Option<u16>,
) {
    assert_eq!(deepkeep_failure(&error).status, status);
}
