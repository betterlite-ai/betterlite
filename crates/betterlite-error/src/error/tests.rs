#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as _;
use std::io::{Error as IoError, ErrorKind};

use rstest::rstest;
use rust_decimal::Decimal;

use super::details::{
    ApiDetails, BudgetExceededDetails, GuardrailDetails, MidStreamFallbackDetails, MidStreamStatus,
    RateLimitDetails, SchemaValidationDetails, TimeoutDetails, TimeoutStatus,
};
use super::{ErrorClass, ErrorContext, TaxonomyError};
use crate::envelope::WireField;
use crate::rate_limit::{RateLimitCategory, RateLimitDimension};

fn context(message: &str) -> ErrorContext {
    ErrorContext {
        message: message.to_owned(),
        model: Some("gpt-4".to_owned()),
        provider: Some("openai".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

fn context_with_body(
    message: &str,
    body_type: &str,
    body_param: Option<&str>,
    body_code: Option<&str>,
) -> ErrorContext {
    ErrorContext {
        body_type: Some(body_type.to_owned()),
        body_param: body_param.map(str::to_owned),
        body_code: body_code.map(str::to_owned),
        ..context(message)
    }
}

#[rstest]
#[case::authentication_type_and_param_are_null(
    ErrorClass::Authentication(Box::new(context("bad key"))),
    401,
    WireField::Null,
    WireField::Null
)]
#[case::not_found_type_and_param_are_null(
    ErrorClass::NotFound(Box::new(context("no such model"))),
    404,
    WireField::Null,
    WireField::Null
)]
#[case::bad_request_type_and_param_are_null(
    ErrorClass::BadRequest(Box::new(context("bad body"))),
    400,
    WireField::Null,
    WireField::Null
)]
#[case::internal_server_sets_a_literal_type(
    ErrorClass::InternalServer(Box::new(context("boom"))),
    500,
    WireField::Text("internal_server_error".to_owned()),
    WireField::Null
)]
#[case::unsupported_params_never_sets_type_but_param_keeps_its_class_default(
    ErrorClass::UnsupportedParams(Box::new(context("unknown kwarg"))),
    400,
    WireField::MISSING,
    WireField::Null
)]
#[case::unclassified_is_a_bare_exception(
    ErrorClass::Unclassified(Box::new(context("caught something unexpected"))),
    500,
    WireField::MISSING,
    WireField::MISSING
)]
#[case::timeout_defaults_to_408(
    ErrorClass::Timeout(Box::new(TimeoutDetails {
        context: context("timed out"),
        original_status: TimeoutStatus(408),
    })),
    408,
    WireField::Null,
    WireField::Null
)]
#[case::timeout_keeps_an_explicit_status_override(
    ErrorClass::Timeout(Box::new(TimeoutDetails {
        context: context("timed out"),
        original_status: TimeoutStatus(504),
    })),
    504,
    WireField::Null,
    WireField::Null
)]
#[case::bad_request_with_a_body_sets_type_and_param_from_it(
    ErrorClass::BadRequest(Box::new(context_with_body(
        "bad body",
        "invalid_request_error",
        Some("messages"),
        None
    ))),
    400,
    WireField::Text("invalid_request_error".to_owned()),
    WireField::Text("messages".to_owned())
)]
#[case::content_policy_violation_with_a_body_but_no_param(
    ErrorClass::ContentPolicyViolation(Box::new(context_with_body(
        "flagged",
        "invalid_request_error",
        None,
        Some("vector_store_search_failed")
    ))),
    400,
    WireField::Text("invalid_request_error".to_owned()),
    WireField::Null
)]
#[case::context_window_exceeded_has_no_body_and_stays_null(
    ErrorClass::ContextWindowExceeded(Box::new(context("too many tokens"))),
    400,
    WireField::Null,
    WireField::Null
)]
#[case::authentication_never_takes_a_body_even_when_one_is_sent(
    ErrorClass::Authentication(Box::new(context_with_body(
        "bad key",
        "invalid_request_error",
        Some("authorization"),
        Some("invalid_api_key")
    ))),
    401,
    WireField::Null,
    WireField::Null
)]
#[case::context_window_exceeded_never_takes_a_body_even_when_one_is_sent(
    ErrorClass::ContextWindowExceeded(Box::new(context_with_body(
        "too many tokens",
        "invalid_request_error",
        Some("messages"),
        Some("context_length_exceeded")
    ))),
    400,
    WireField::Null,
    WireField::Null
)]
#[case::internal_server_with_a_body_takes_param_but_keeps_its_literal_type(
    ErrorClass::InternalServer(Box::new(context_with_body(
        "boom",
        "server_error",
        Some("model"),
        Some("upstream_failure")
    ))),
    500,
    WireField::Text("internal_server_error".to_owned()),
    WireField::Text("model".to_owned())
)]
#[case::rate_limit_with_a_body_takes_param_but_keeps_its_literal_type(
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context_with_body(
            "slow down",
            "server_error",
            Some("retry_after"),
            Some("vendor_throttled")
        ),
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    })),
    429,
    WireField::Text("throttling_error".to_owned()),
    WireField::Text("retry_after".to_owned())
)]
#[case::api_error_with_a_body_sets_type_and_param_from_it(
    ErrorClass::Api(Box::new(ApiDetails {
        context: context_with_body("vendor failure", "server_error", Some("model"), None),
        status: 500,
    })),
    500,
    WireField::Text("server_error".to_owned()),
    WireField::Text("model".to_owned())
)]
#[case::rate_limit_sets_the_throttling_type(
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context("slow down"),
        category: RateLimitCategory::VendorRateLimit,
        dimension: Some(RateLimitDimension::Requests),
        headers: BTreeMap::new(),
    })),
    429,
    WireField::Text("throttling_error".to_owned()),
    WireField::Null
)]
#[case::api_error_keeps_its_given_status(
    ErrorClass::Api(Box::new(ApiDetails {
        context: context("vendor failure"),
        status: 418,
    })),
    418,
    WireField::Null,
    WireField::Null
)]
#[case::rate_limit_debug_hides_cookie_and_secret_header_values(
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context("slow down"),
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::from([
            ("set-cookie".to_owned(), "session=leaked-cookie-value".to_owned()),
            ("x-vendor-secret".to_owned(), "sk-leaked-secret-token".to_owned()),
        ]),
    })),
    429,
    WireField::Text("throttling_error".to_owned()),
    WireField::Null
)]
fn error_types_error_type_tests_behavior(
    #[case] class: ErrorClass,
    #[case] expected_status: u16,
    #[case] expected_type: WireField,
    #[case] expected_param: WireField,
) {
    let debug_text = format!("{class:?}");
    assert!(!debug_text.contains("leaked-cookie-value"));
    assert!(!debug_text.contains("leaked-secret-token"));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, expected_status);
    assert_eq!(metadata.error_type, expected_type);
    assert_eq!(metadata.param, expected_param);
    assert_eq!(metadata.code, expected_status.to_string());
}

#[test]
fn bad_request_with_a_body_code_keeps_it_as_openai_code() {
    let class = ErrorClass::BadRequest(Box::new(context_with_body(
        "vector store search failed",
        "invalid_request_error",
        None,
        Some("vector_store_search_failed"),
    )));
    let metadata = class.metadata();
    assert_eq!(
        metadata.openai_code,
        Some("vector_store_search_failed".to_owned())
    );
}

#[test]
fn bad_request_without_a_body_leaves_openai_code_absent() {
    let class = ErrorClass::BadRequest(Box::new(context("bad body")));
    assert_eq!(class.metadata().openai_code, None);
}

#[test]
fn authentication_with_a_body_code_never_sets_openai_code() {
    let class = ErrorClass::Authentication(Box::new(context_with_body(
        "bad key",
        "invalid_request_error",
        None,
        Some("invalid_api_key"),
    )));
    assert_eq!(class.metadata().openai_code, None);
}

#[test]
fn context_window_exceeded_with_a_body_code_never_sets_openai_code() {
    let class = ErrorClass::ContextWindowExceeded(Box::new(context_with_body(
        "too many tokens",
        "invalid_request_error",
        None,
        Some("context_length_exceeded"),
    )));
    assert_eq!(class.metadata().openai_code, None);
}

#[test]
fn internal_server_with_a_body_code_keeps_it_as_openai_code() {
    let class = ErrorClass::InternalServer(Box::new(context_with_body(
        "boom",
        "server_error",
        None,
        Some("upstream_failure"),
    )));
    assert_eq!(
        class.metadata().openai_code,
        Some("upstream_failure".to_owned())
    );
}

#[test]
fn internal_server_without_a_body_leaves_openai_code_absent() {
    let class = ErrorClass::InternalServer(Box::new(context("boom")));
    assert_eq!(class.metadata().openai_code, None);
}

#[test]
fn rate_limit_carries_its_category_and_dimension() {
    let class = ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context("slow down"),
        category: RateLimitCategory::LitellmBatchRateLimit,
        dimension: Some(RateLimitDimension::Budget),
        headers: BTreeMap::new(),
    }));
    let metadata = class.metadata();
    assert_eq!(
        metadata.rate_limit_category,
        Some(RateLimitCategory::LitellmBatchRateLimit)
    );
    assert_eq!(
        metadata.rate_limit_dimension,
        Some(RateLimitDimension::Budget)
    );
}

#[test]
fn rate_limit_openai_code_is_always_the_429_literal() {
    let class = ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context("slow down"),
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    }));
    assert_eq!(class.metadata().openai_code, Some("429".to_owned()));
}

#[test]
fn rate_limit_openai_code_stays_429_even_when_a_body_code_is_sent() {
    let class = ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context_with_body("slow down", "server_error", None, Some("vendor_throttled")),
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    }));
    assert_eq!(class.metadata().openai_code, Some("429".to_owned()));
}

#[test]
fn rate_limit_metadata_drops_unsafe_headers_before_they_become_allowed() {
    let headers = BTreeMap::from([
        ("retry-after".to_owned(), "5".to_owned()),
        ("set-cookie".to_owned(), "a=b".to_owned()),
    ]);
    let class = ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context("slow down"),
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers,
    }));
    let metadata = class.metadata();
    assert_eq!(
        metadata.allowed_response_headers,
        BTreeMap::from([("retry-after".to_owned(), "5".to_owned())])
    );
}

#[test]
fn rate_limit_details_debug_names_the_header_but_not_its_secret_value() {
    let details = RateLimitDetails {
        context: context("slow down"),
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::from([(
            "set-cookie".to_owned(),
            "session=leaked-cookie-value".to_owned(),
        )]),
    };
    let debug_text = format!("{details:?}");
    assert!(debug_text.contains("set-cookie"));
    assert!(!debug_text.contains("leaked-cookie-value"));
}

#[test]
fn budget_exceeded_uses_the_configured_status_and_marks_the_budget_dimension() {
    let class = ErrorClass::BudgetExceeded(Box::new(BudgetExceededDetails {
        current_cost: Decimal::new(125, 1),
        max_budget: Decimal::from(10_i32),
        message: None,
        provider: "openai".to_owned(),
        entity_type: Some("key".to_owned()),
        entity_id: Some("key-1".to_owned()),
        status_code: 422,
    }));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 422);
    assert_eq!(metadata.error_type, WireField::MISSING);
    assert_eq!(metadata.provider, Some("openai".to_owned()));
    assert_eq!(
        metadata.rate_limit_category,
        Some(RateLimitCategory::LitellmRateLimit)
    );
    assert_eq!(
        metadata.rate_limit_dimension,
        Some(RateLimitDimension::Budget)
    );
}

#[test]
fn budget_exceeded_without_a_provider_reports_an_empty_string_like_litellm() {
    let class = ErrorClass::BudgetExceeded(Box::new(BudgetExceededDetails {
        current_cost: Decimal::new(125, 1),
        max_budget: Decimal::from(10_i32),
        message: None,
        provider: String::new(),
        entity_type: None,
        entity_id: None,
        status_code: 422,
    }));
    let metadata = class.metadata();
    assert_eq!(metadata.provider, Some(String::new()));
}

#[test]
fn guardrail_raised_never_sets_type_or_param() {
    let class = ErrorClass::GuardrailRaised(Box::new(GuardrailDetails {
        guardrail_name: Some("pii".to_owned()),
        message: "blocked".to_owned(),
        status_code: 400,
        blocked_content: true,
    }));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.error_type, WireField::MISSING);
    assert_eq!(metadata.param, WireField::MISSING);
}

#[test]
fn mid_stream_fallback_wire_input_without_a_status_defaults_to_503() {
    let json = r#"{
        "family": "mid_stream_fallback",
        "context": {"message": "fallback", "model": "gpt-4", "provider": "openai"},
        "generated_content": "",
        "is_pre_first_chunk": true
    }"#;
    let class: ErrorClass = serde_json::from_str(json).expect("deserializes without a status");
    assert_eq!(class.metadata().status_code, 503);
}

#[test]
fn mid_stream_fallback_keeps_the_original_exception_status() {
    let class = ErrorClass::MidStreamFallback(Box::new(MidStreamFallbackDetails {
        context: context("fallback"),
        original_status: MidStreamStatus(529),
        generated_content: "partial".to_owned(),
        is_pre_first_chunk: false,
    }));
    assert_eq!(class.metadata().status_code, 529);
}

#[test]
fn timeout_wire_input_without_a_status_defaults_to_408() {
    let json = r#"{
        "family": "timeout",
        "context": {"message": "timed out", "model": "gpt-4", "provider": "openai"}
    }"#;
    let class: ErrorClass = serde_json::from_str(json).expect("deserializes without a status");
    assert_eq!(class.metadata().status_code, 408);
}

#[test]
fn json_schema_validation_keeps_the_model_and_provider_identity() {
    let class = ErrorClass::JsonSchemaValidation(Box::new(SchemaValidationDetails {
        model: "gpt-4".to_owned(),
        provider: "openai".to_owned(),
        raw_response: "{\"bad\": true}".to_owned(),
        schema: "{\"type\": \"object\"}".to_owned(),
    }));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 500);
    assert_eq!(metadata.model, Some("gpt-4".to_owned()));
    assert_eq!(metadata.provider, Some("openai".to_owned()));
}

fn timeout_cause() -> IoError {
    IoError::new(ErrorKind::TimedOut, "deadline exceeded")
}

fn transport_cause() -> IoError {
    IoError::new(ErrorKind::ConnectionRefused, "connection refused")
}

fn parse_cause(marker: &str) -> serde_json::Error {
    serde_json::from_str::<u32>(&format!("\"{marker}\"")).unwrap_err()
}

#[test]
fn taxonomy_timeout_preserves_its_source() {
    let error = TaxonomyError::Timeout {
        provider: "openai".to_owned(),
        model: Some("gpt-4".to_owned()),
        cause: timeout_cause(),
    };
    let source = error.source().expect("a timeout keeps its cause");
    let io_error = source
        .downcast_ref::<IoError>()
        .expect("the cause stays a std::io::Error");
    assert_eq!(io_error.kind(), ErrorKind::TimedOut);
}

#[test]
fn taxonomy_transport_preserves_its_source() {
    let error = TaxonomyError::Transport {
        provider: "anthropic".to_owned(),
        model: None,
        cause: transport_cause(),
    };
    let source = error.source().expect("a transport failure keeps its cause");
    let io_error = source
        .downcast_ref::<IoError>()
        .expect("the cause stays a std::io::Error");
    assert_eq!(io_error.kind(), ErrorKind::ConnectionRefused);
}

#[test]
fn taxonomy_parse_preserves_its_source() {
    let cause = parse_cause("unexpected-end-of-json");
    let error = TaxonomyError::Parse {
        provider: "azure".to_owned(),
        model: None,
        cause,
    };
    let source = error.source().expect("a parse failure keeps its cause");
    assert!(source.to_string().contains("unexpected-end-of-json"));
}

#[test]
fn taxonomy_client_message_never_includes_the_cause_text() {
    let secret = "super-secret-upstream-marker";
    let cause = parse_cause(secret);
    let error = TaxonomyError::Parse {
        provider: "openai".to_owned(),
        model: None,
        cause,
    };
    let class = error.classify();
    let ErrorClass::ApiResponseValidation(context) = &class else {
        panic!("Parse classifies as ApiResponseValidation");
    };
    assert!(!context.message.contains(secret));
    assert!(!error.to_string().contains(secret));
}
