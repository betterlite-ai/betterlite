#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as _;

use rstest::rstest;
use rust_decimal::Decimal;

use super::details::{
    ApiDetails, BudgetExceededDetails, GuardrailDetails, MidStreamFallbackDetails,
    RateLimitDetails, SchemaValidationDetails, TimeoutDetails,
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
        original_status: None,
    })),
    408,
    WireField::Null,
    WireField::Null
)]
#[case::timeout_keeps_an_explicit_status_override(
    ErrorClass::Timeout(Box::new(TimeoutDetails {
        context: context("timed out"),
        original_status: Some(504),
    })),
    504,
    WireField::Null,
    WireField::Null
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
fn error_types_error_type_tests_behavior(
    #[case] class: ErrorClass,
    #[case] expected_status: u16,
    #[case] expected_type: WireField,
    #[case] expected_param: WireField,
) {
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, expected_status);
    assert_eq!(metadata.error_type, expected_type);
    assert_eq!(metadata.param, expected_param);
    assert_eq!(metadata.code, WireField::Text(expected_status.to_string()));
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
fn budget_exceeded_uses_the_configured_status_and_marks_the_budget_dimension() {
    let class = ErrorClass::BudgetExceeded(Box::new(BudgetExceededDetails {
        current_cost: Decimal::new(125, 1),
        max_budget: Decimal::from(10_i32),
        message: None,
        provider: Some("openai".to_owned()),
        entity_type: Some("key".to_owned()),
        entity_id: Some("key-1".to_owned()),
        status_code: 422,
    }));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 422);
    assert_eq!(metadata.error_type, WireField::MISSING);
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
fn mid_stream_fallback_falls_back_to_503_without_an_original_status() {
    let class = ErrorClass::MidStreamFallback(Box::new(MidStreamFallbackDetails {
        context: context("fallback"),
        original_status: None,
        generated_content: String::new(),
        is_pre_first_chunk: true,
    }));
    assert_eq!(class.metadata().status_code, 503);
}

#[test]
fn mid_stream_fallback_keeps_the_original_exception_status() {
    let class = ErrorClass::MidStreamFallback(Box::new(MidStreamFallbackDetails {
        context: context("fallback"),
        original_status: Some(529),
        generated_content: "partial".to_owned(),
        is_pre_first_chunk: false,
    }));
    assert_eq!(class.metadata().status_code, 529);
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

// a builder error from an unparseable port: synchronous, no network, and a real reqwest::Error
fn transport_cause() -> reqwest::Error {
    reqwest::Client::default()
        .get("http://host:not-a-port/")
        .build()
        .unwrap_err()
}

fn parse_cause(marker: &str) -> serde_json::Error {
    serde_json::from_str::<u32>(&format!("\"{marker}\"")).unwrap_err()
}

#[test]
fn taxonomy_timeout_preserves_its_source() {
    let error = TaxonomyError::Timeout {
        provider: "openai".to_owned(),
        model: Some("gpt-4".to_owned()),
        cause: transport_cause(),
    };
    let source = error.source().expect("a timeout keeps its cause");
    let reqwest_error = source
        .downcast_ref::<reqwest::Error>()
        .expect("the cause stays a reqwest::Error");
    assert!(reqwest_error.is_builder());
}

#[test]
fn taxonomy_transport_preserves_its_source() {
    let error = TaxonomyError::Transport {
        provider: "anthropic".to_owned(),
        model: None,
        cause: transport_cause(),
    };
    let source = error.source().expect("a transport failure keeps its cause");
    let reqwest_error = source
        .downcast_ref::<reqwest::Error>()
        .expect("the cause stays a reqwest::Error");
    assert!(reqwest_error.is_builder());
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
