use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::datadog::apm::NullTracer;
use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::payloads::types::DatadogPayload;
use crate::datadog::proxy_failure::async_post_call_failure_hook;

use super::support::{
    Intake, build_logger, exception, exception_logging, field, json, last_queued, message,
    normalizer, text,
};

async fn logged_proxy_failure(encoded: Option<&str>, traceback: Option<&str>) -> DatadogPayload {
    let (logger, _) = build_logger(&[], Intake::Accepted);
    let patterns = normalizer();
    let facts = encoded.map(exception);
    let context = exception_logging(facts.as_ref(), &patterns);
    async_post_call_failure_hook(&logger, &NullTracer, &context, traceback, IndexMap::new()).await;
    last_queued(&logger)
}

#[tokio::test]
async fn datadog_proxy_error_extraction_behavior() {
    // the code wins over status_code, and a numeric code becomes the HTTP status
    let coded = logged_proxy_failure(
        Some(r#"{"class_name": "BadRequestError", "code": "418", "status_code": 400, "message": "bad input"}"#),
        None,
    )
    .await;
    assert_eq!(coded.status, DataDogStatus::Error);
    let coded_body = message(&coded);
    assert_eq!(text(field(&coded_body, "exception")), "bad input");
    assert_eq!(text(field(&coded_body, "error_class")), "BadRequestError");
    assert_eq!(field(&coded_body, "status_code"), &json("418"));
    assert_eq!(text(field(&coded_body, "traceback")), "");
    assert_eq!(field(&coded_body, "user_api_key_dict"), &json("{}"));

    // without a code the status code is the one to log, and the traceback argument starts the text
    let status_only = logged_proxy_failure(
        Some(r#"{"class_name": "APIConnectionError", "status_code": 503, "message": "connection failed"}"#),
        Some("trace text"),
    )
    .await;
    let status_only_body = message(&status_only);
    assert_eq!(
        text(field(&status_only_body, "exception")),
        "connection failed"
    );
    assert_eq!(field(&status_only_body, "status_code"), &json("503"));
    assert_eq!(text(field(&status_only_body, "traceback")), "trace text");

    // a non-numeric code leaves status_code null, even when the exception carries a status
    let named = logged_proxy_failure(
        Some(r#"{"class_name": "RateLimitError", "code": "rate_limited", "status_code": 429, "message": "slow down"}"#),
        None,
    )
    .await;
    let named_body = message(&named);
    assert_eq!(text(field(&named_body, "error_class")), "RateLimitError");
    assert_eq!(field(&named_body, "status_code"), &Json::Null);
}

#[tokio::test]
async fn datadog_proxy_trimmed_code_and_missing_exception_behavior() {
    // the code is stripped before its digits are read, and the stored traceback lines follow the argument
    let padded = logged_proxy_failure(
        Some(r#"{"class_name": "AuthError", "code": " 401 ", "message": "no key", "traceback_lines": ["first\n", "second\n"]}"#),
        None,
    )
    .await;
    let padded_body = message(&padded);
    assert_eq!(field(&padded_body, "status_code"), &json("401"));
    assert_eq!(text(field(&padded_body, "traceback")), "first\nsecond\n");

    // an absent exception still produces every field
    let absent = logged_proxy_failure(None, None).await;
    let absent_body = message(&absent);
    assert_eq!(text(field(&absent_body, "exception")), "");
    assert_eq!(text(field(&absent_body, "error_class")), "");
    assert_eq!(field(&absent_body, "status_code"), &Json::Null);
    assert_eq!(text(field(&absent_body, "traceback")), "");
    assert_eq!(field(&absent_body, "user_api_key_dict"), &json("{}"));
}
