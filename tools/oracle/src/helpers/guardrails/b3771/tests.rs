#![cfg(test)]
use std::net::TcpListener;

use betterlite_guardrails::generic_guardrail_api::GenericGuardrailAPIError;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailExtraValue;
use betterlite_guardrails::grayswan::error::GraySwanGuardrailAPIError;
use reqwest::StatusCode;
use reqwest::header::HeaderValue;

use super::generic::result_payload;
use super::grayswan::failure;

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

fn blocked() -> GraySwanGuardrailAPIError {
    GraySwanGuardrailAPIError::Blocked {
        reason: String::new(),
        violation_location: "input",
        violation: 0.9_f64,
        violated_rules: Vec::<GuardrailExtraValue>::new(),
        mutation: None,
        ipi: None,
    }
}

fn timeout() -> GraySwanGuardrailAPIError {
    GraySwanGuardrailAPIError::Timeout {
        source: connection_refused(),
    }
}

fn http_status() -> GraySwanGuardrailAPIError {
    GraySwanGuardrailAPIError::HttpStatus {
        status: StatusCode::NOT_FOUND,
    }
}

fn request() -> GraySwanGuardrailAPIError {
    GraySwanGuardrailAPIError::Request {
        source: connection_refused(),
    }
}

fn invalid_header() -> GraySwanGuardrailAPIError {
    GraySwanGuardrailAPIError::InvalidHeader {
        source: HeaderValue::from_str("\u{0}").unwrap_err(),
    }
}

fn response_json() -> GraySwanGuardrailAPIError {
    GraySwanGuardrailAPIError::ResponseJson {
        source: serde_json::from_str::<serde_json::Value>("not json").unwrap_err(),
    }
}

#[test]
fn grayswan_failures_carry_the_python_status() {
    assert_eq!(failure(&blocked()).status, Some(400));
    assert_eq!(failure(&timeout()).status, Some(408));
    assert_eq!(failure(&http_status()).status, Some(404));
    assert_eq!(failure(&request()).status, None);
    assert_eq!(failure(&invalid_header()).status, None);
    assert_eq!(
        failure(&GraySwanGuardrailAPIError::ResponseTooLarge { limit: 10 }).status,
        None
    );
    assert_eq!(failure(&response_json()).status, None);
}

#[test]
fn generic_block_carries_status_400() {
    let Err(blocked_failure) = result_payload(Err(GenericGuardrailAPIError::Blocked {
        reason: String::new(),
    })) else {
        panic!("expected an error result");
    };
    assert_eq!(blocked_failure.status, Some(400));

    let Err(call_failure) = result_payload(Err(GenericGuardrailAPIError::Request {
        source: connection_refused(),
    })) else {
        panic!("expected an error result");
    };
    assert_eq!(call_failure.status, None);
}
