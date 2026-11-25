#![cfg(test)]

use super::super::NativeImageError;
use super::{CallFailure, SessionError, class_error, context, convert_failure, provider_failure};
use crate::images::convert::ConvertError;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::RateLimitDetails;
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct ErrorPayload {
    #[serde(rename = "type")]
    kind: String,
    status_code: u16,
    headers: BTreeMap<String, String>,
    num_retries: Option<u32>,
    max_retries: Option<u32>,
    message: String,
    model: String,
    llm_provider: String,
}

#[test]
fn plain_failure_keeps_kind_message_and_model() {
    let failure = CallFailure::plain(
        "ValueError",
        String::from("no image"),
        Some(String::from("dall-e-2")),
    );
    assert_eq!(failure.kind, "ValueError");
    assert_eq!(failure.message, "no image");
    assert_eq!(failure.model.as_deref(), Some("dall-e-2"));
    assert_eq!(failure.status_code, None);
}

#[test]
fn connection_failure_serializes_class_metadata() {
    Python::initialize();
    Python::attach(|py| {
        let mut details = context(
            String::from("connection refused"),
            Some(String::from("dall-e-2")),
            Some(String::from("openai")),
        );
        details.num_retries = Some(3);
        details.max_retries = Some(5);
        let failure = CallFailure::from_class(&ErrorClass::ApiConnection(Box::new(details)));
        let error = class_error(&failure);
        let text = error.value(py).str().unwrap();
        let payload: ErrorPayload = serde_json::from_str(text.to_str().unwrap()).unwrap();
        assert_eq!(payload.kind, "APIConnectionError");
        assert_eq!(payload.status_code, 500);
        assert!(payload.headers.is_empty());
        assert_eq!(payload.num_retries, Some(3));
        assert_eq!(payload.max_retries, Some(5));
        assert_eq!(payload.message, "connection refused");
        assert_eq!(payload.model, "dall-e-2");
        assert_eq!(payload.llm_provider, "openai");
        assert!(error.is_instance_of::<NativeImageError>(py));
    });
}

#[test]
fn provider_failure_keeps_status_and_provider_context() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let failure = provider_failure(
        String::from("rate limited"),
        Some(429),
        "dall-e-2",
        "openai",
        &redactor,
    );
    assert_eq!(
        failure.message,
        "RateLimitError: OpenAIException - rate limited"
    );
    assert_eq!(failure.model.as_deref(), Some("dall-e-2"));
    assert_eq!(failure.provider.as_deref(), Some("openai"));
    assert_eq!(failure.kind, "RateLimitError");
}

#[test]
fn unsupported_parameter_conversion_uses_unsupported_class() {
    let error = ConvertError::UnsupportedParams {
        model: String::from("dall-e-2"),
        params: String::from("imageConfig"),
    };
    let failure = convert_failure(
        &error,
        Some(String::from("dall-e-2")),
        Some(String::from("openai")),
    );
    assert_eq!(
        failure.message,
        "The following parameters are not supported for model dall-e-2: imageConfig"
    );
    assert_eq!(failure.kind, "UnsupportedParamsError");
    assert_eq!(failure.status_code, Some(400));
}

#[test]
fn invalid_session_json_maps_to_value_error() {
    Python::initialize();
    Python::attach(|py| {
        let source = serde_json::from_str::<String>("not-json").unwrap_err();
        let error = PyErr::from(SessionError::Json(ConvertError::Json { source }));
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn rate_limit_payload_keeps_headers_and_status() {
    let class = ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context(
            String::from("retry later"),
            Some(String::from("dall-e-2")),
            Some(String::from("openai")),
        ),
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::from([(String::from("retry-after"), String::from("17"))]),
    }));
    let failure = CallFailure::from_class(&class);
    let payload: ErrorPayload =
        serde_json::from_str(&serde_json::to_string(&failure).unwrap()).unwrap();
    assert_eq!(payload.kind, "RateLimitError");
    assert_eq!(payload.status_code, 429);
    assert_eq!(
        payload.headers.get("retry-after").map(String::as_str),
        Some("17")
    );
}
