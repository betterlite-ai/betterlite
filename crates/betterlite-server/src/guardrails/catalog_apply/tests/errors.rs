use std::collections::BTreeMap;
use std::io::Error as IoError;

use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;

use super::super::error::CatalogApplyError;
use super::support::body_value;
use super::value;
use crate::guardrails::callback_caps::errors::{ProxyException, ProxyHookError};

fn proxy_exception() -> CatalogApplyError {
    CatalogApplyError::Hooks(ProxyHookError::Proxy(Box::new(ProxyException {
        message: "rate limited".to_owned(),
        error_type: "RateLimitError".to_owned(),
        param: Some("key".to_owned()),
        code: "429".to_owned(),
        headers: BTreeMap::new(),
    })))
}

#[test]
fn guardrail_failure_status_comes_from_the_failure() {
    let error = CatalogApplyError::Guardrail(GuardrailFailure::Http {
        status: 403,
        detail: "blocked".to_owned(),
    });
    assert_eq!(error.status(), StatusCode::FORBIDDEN);
}

#[test]
fn hook_error_status_comes_from_the_hook_error() {
    let error = CatalogApplyError::Hooks(ProxyHookError::Http {
        status: 401,
        detail: "denied".to_owned(),
    });
    assert_eq!(error.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn category_read_keeps_the_cause_in_the_detail() {
    let error = CatalogApplyError::CategoryRead {
        source: IoError::other("disk on fire"),
    };
    assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(error.detail(), "Error reading category file: disk on fire");
}

#[test]
fn airlines_read_keeps_the_parser_message_in_the_detail() {
    let source = serde_json::from_str::<Vec<u8>>("not json").unwrap_err();
    let message = source.to_string();
    let error = CatalogApplyError::AirlinesRead { source };
    assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.detail(),
        format!("Error reading major_airlines.json: {message}")
    );
}

#[test]
fn proxy_exception_status_comes_from_its_numeric_code() {
    assert_eq!(proxy_exception().status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn proxy_exception_answers_the_proxy_error_body_with_its_param() {
    let response = proxy_exception().into_response();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        body_value(response).await,
        value(
            r#"{"error":{"message":"rate limited","type":"RateLimitError","param":"key","code":"429"}}"#
        )
    );
}
