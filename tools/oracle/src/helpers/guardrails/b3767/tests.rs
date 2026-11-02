#![cfg(test)]
use std::net::TcpListener;

use betterlite_guardrails::deepkeep::error::DeepKeepError;
use reqwest::StatusCode;
use reqwest::header::{HeaderName, HeaderValue};

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

fn blocked() -> DeepKeepError {
    DeepKeepError::Blocked {
        reason: "policy".to_owned(),
    }
}

fn api() -> DeepKeepError {
    DeepKeepError::Api {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        body: "boom".to_owned(),
    }
}

fn api_encoding() -> DeepKeepError {
    DeepKeepError::ApiEncoding {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        source: utf8_error(),
    }
}

fn response_json() -> DeepKeepError {
    DeepKeepError::ResponseJson {
        source: serde_json::from_str::<serde_json::Value>("not json").unwrap_err(),
    }
}

fn response_too_large() -> DeepKeepError {
    DeepKeepError::ResponseTooLarge { limit: 10 }
}

fn request() -> DeepKeepError {
    DeepKeepError::Request {
        source: connection_refused(),
    }
}

fn timeout() -> DeepKeepError {
    DeepKeepError::Timeout {
        source: connection_refused(),
    }
}

fn request_build() -> DeepKeepError {
    DeepKeepError::RequestBuild {
        source: connection_refused(),
    }
}

fn invalid_header() -> DeepKeepError {
    DeepKeepError::InvalidHeader {
        name: "deepkeep-api-key",
        source: HeaderValue::from_str("\u{0}").unwrap_err(),
    }
}

fn invalid_header_name() -> DeepKeepError {
    DeepKeepError::InvalidHeaderName {
        name: "\u{0}".to_owned(),
        source: HeaderName::from_bytes(b"\0").unwrap_err(),
    }
}

#[test]
fn deepkeep_failures_carry_the_python_status() {
    assert_eq!(deepkeep_failure(&blocked()).status, Some(400));
    assert_eq!(deepkeep_failure(&api()).status, None);
    assert_eq!(deepkeep_failure(&api_encoding()).status, None);
    assert_eq!(deepkeep_failure(&response_json()).status, None);
    assert_eq!(deepkeep_failure(&response_too_large()).status, None);
    assert_eq!(deepkeep_failure(&request()).status, None);
    assert_eq!(deepkeep_failure(&timeout()).status, None);
    assert_eq!(deepkeep_failure(&request_build()).status, None);
    assert_eq!(deepkeep_failure(&invalid_header()).status, None);
    assert_eq!(deepkeep_failure(&invalid_header_name()).status, None);
}
