use betterlite_models::decisions::Json;
use betterlite_sdk::completion_request::call::PreparedTimeout;
use betterlite_sdk::completion_request::error::CompletionRequestError;
use betterlite_sdk::completion_request::ports::{CompletionTimeout as _, TimeoutRequest};

use super::{SessionTimeouts, number_of};

fn value(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn resolved(
    model_timeout: Option<&Json>,
    timeout: Option<&Json>,
    request_timeout: Option<&Json>,
    configured: Option<f64>,
) -> PreparedTimeout {
    let request = TimeoutRequest {
        model_timeout,
        timeout,
        request_timeout,
        configured_request_timeout: configured,
        custom_llm_provider: "openai",
    };
    SessionTimeouts.resolve(&request).unwrap()
}

#[test]
fn timeout_resolution_prefers_the_model_timeout() {
    let model_timeout = value("5");
    let timeout = value("6");
    let request_timeout = value("7");
    assert_eq!(
        resolved(
            Some(&model_timeout),
            Some(&timeout),
            Some(&request_timeout),
            Some(9.0_f64)
        ),
        PreparedTimeout::Seconds(5.0_f64)
    );
}

#[test]
fn timeout_resolution_prefers_the_timeout_kwarg_then_request_timeout() {
    let timeout = value("6");
    let request_timeout = value("7");
    assert_eq!(
        resolved(None, Some(&timeout), Some(&request_timeout), Some(9.0_f64)),
        PreparedTimeout::Seconds(6.0_f64)
    );
    assert_eq!(
        resolved(None, None, Some(&request_timeout), Some(9.0_f64)),
        PreparedTimeout::Seconds(7.0_f64)
    );
}

#[test]
fn timeout_resolution_falls_back_to_the_configured_value_then_600() {
    assert_eq!(
        resolved(None, None, None, Some(9.0_f64)),
        PreparedTimeout::Seconds(9.0_f64)
    );
    assert_eq!(
        resolved(None, None, None, None),
        PreparedTimeout::Seconds(600.0_f64)
    );
    assert_eq!(
        resolved(None, None, None, Some(-1.0_f64)),
        PreparedTimeout::Seconds(600.0_f64)
    );
}

#[test]
fn a_timeout_mapping_collapses_to_its_read_phase() {
    let mapping = value(r#"{"connect": 2, "read": 30}"#);
    assert_eq!(
        resolved(Some(&mapping), None, None, None),
        PreparedTimeout::Seconds(30.0_f64)
    );
    let without_read = value(r#"{"connect": 2}"#);
    assert_eq!(
        resolved(Some(&without_read), None, None, None),
        PreparedTimeout::Seconds(600.0_f64)
    );
}

#[test]
fn a_timeout_that_is_not_a_number_of_seconds_is_a_field_error() {
    let negative = value("-1");
    let error = number_of(&negative, "timeout").unwrap_err();
    assert!(matches!(
        error,
        CompletionRequestError::FieldType {
            field: "timeout",
            ..
        }
    ));
    let text = value("\"soon\"");
    assert!(number_of(&text, "timeout").is_err());
    let numeric_text = value("\"30\"");
    assert_eq!(number_of(&numeric_text, "timeout").unwrap(), 30.0_f64);
}
