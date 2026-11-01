#![cfg(test)]
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::time::Duration;

use betterlite_guardrails::lasso::error::LassoError;
use betterlite_guardrails::lasso::wire::LassoResponse;
use reqwest::StatusCode;
use reqwest::header::HeaderValue;
use rstest::rstest;

use super::lasso_failure;

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

fn response_timeout() -> reqwest::Error {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        client.get(format!("http://{addr}")).send().await
    });
    drop(listener);
    result.unwrap_err()
}

#[rstest]
#[case::blocked(
    LassoError::Blocked {
        deputies: vec!["toxicity".to_owned()],
        response: LassoResponse {
            violations_detected: true,
            deputies: BTreeMap::new(),
            findings: BTreeMap::new(),
        },
    },
    Some(400)
)]
#[case::invalid_api_key(LassoError::InvalidApiKey, None)]
#[case::rate_limited(LassoError::RateLimited, None)]
#[case::http_status(
    LassoError::HttpStatus {
        status: StatusCode::INTERNAL_SERVER_ERROR,
    },
    None
)]
#[case::response_too_large(LassoError::ResponseTooLarge { limit: 10 }, None)]
#[case::invalid_header(
    LassoError::InvalidHeader {
        name: "lasso-api-key",
        source: HeaderValue::from_str("\u{0}").unwrap_err(),
    },
    None
)]
#[case::response_json(
    LassoError::ResponseJson {
        source: serde_json::from_str::<serde_json::Value>("not json").unwrap_err(),
    },
    None
)]
#[case::request(LassoError::Request { source: connection_refused() }, None)]
#[case::timeout(LassoError::Timeout { source: response_timeout() }, None)]
fn lasso_failures_carry_the_python_status(#[case] error: LassoError, #[case] status: Option<u16>) {
    assert_eq!(lasso_failure(&error).status, status);
}
