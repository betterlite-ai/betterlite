#![cfg(test)]
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::time::Duration;

use betterlite_guardrails::lasso::error::LassoError;
use betterlite_guardrails::lasso::wire::LassoResponse;
use reqwest::StatusCode;
use reqwest::header::HeaderValue;

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

fn blocked() -> LassoError {
    LassoError::Blocked {
        deputies: vec!["toxicity".to_owned()],
        response: LassoResponse {
            violations_detected: true,
            deputies: BTreeMap::new(),
            findings: BTreeMap::new(),
        },
    }
}

fn http_status() -> LassoError {
    LassoError::HttpStatus {
        status: StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn invalid_header() -> LassoError {
    LassoError::InvalidHeader {
        name: "lasso-api-key",
        source: HeaderValue::from_str("\u{0}").unwrap_err(),
    }
}

fn response_json() -> LassoError {
    LassoError::ResponseJson {
        source: serde_json::from_str::<serde_json::Value>("not json").unwrap_err(),
    }
}

fn response_too_large() -> LassoError {
    LassoError::ResponseTooLarge { limit: 10 }
}

fn request() -> LassoError {
    LassoError::Request {
        source: connection_refused(),
    }
}

fn timeout() -> LassoError {
    LassoError::Timeout {
        source: response_timeout(),
    }
}

#[test]
fn lasso_failures_carry_the_python_status() {
    assert_eq!(lasso_failure(&blocked()).status, Some(400));
    assert_eq!(lasso_failure(&LassoError::InvalidApiKey).status, None);
    assert_eq!(lasso_failure(&LassoError::RateLimited).status, None);
    assert_eq!(lasso_failure(&http_status()).status, None);
    assert_eq!(lasso_failure(&response_too_large()).status, None);
    assert_eq!(lasso_failure(&invalid_header()).status, None);
    assert_eq!(lasso_failure(&response_json()).status, None);
    assert_eq!(lasso_failure(&request()).status, None);
    assert_eq!(lasso_failure(&timeout()).status, None);
}
