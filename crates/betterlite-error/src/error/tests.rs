#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;

use rstest::rstest;

use super::{
    ApiDetails, BudgetExceededDetails, ErrorClass, ErrorContext, GuardrailDetails,
    MidStreamFallbackDetails, RateLimitDetails, SchemaValidationDetails, TaxonomyError,
    TimeoutDetails,
};
use crate::envelope::WireField;
use crate::rate_limit::{RateLimitCategory, RateLimitDimension};

fn context(message: &str) -> ErrorContext {
    ErrorContext::new(
        message.to_owned(),
        Some("gpt-4".to_owned()),
        Some("openai".to_owned()),
    )
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
    WireField::text("internal_server_error".to_owned()),
    WireField::Null
)]
#[case::unsupported_params_never_sets_type_or_param(
    ErrorClass::UnsupportedParams(Box::new(context("unknown kwarg"))),
    400,
    WireField::Missing,
    WireField::Missing
)]
#[case::unclassified_is_a_bare_exception(
    ErrorClass::Unclassified(Box::new(context("caught something unexpected"))),
    500,
    WireField::Missing,
    WireField::Missing
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
    WireField::text("throttling_error".to_owned()),
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
    assert_eq!(metadata.code, WireField::text(expected_status.to_string()));
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
        current_cost: 12.5,
        max_budget: 10.0,
        message: None,
        provider: Some("openai".to_owned()),
        entity_type: Some("key".to_owned()),
        entity_id: Some("key-1".to_owned()),
        status_code: 422,
    }));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 422);
    assert_eq!(metadata.error_type, WireField::Missing);
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
    assert_eq!(metadata.error_type, WireField::Missing);
    assert_eq!(metadata.param, WireField::Missing);
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

#[derive(Debug)]
struct UpstreamFailure(&'static str);

impl fmt::Display for UpstreamFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl StdError for UpstreamFailure {}

#[test]
fn taxonomy_timeout_preserves_its_source() {
    let cause: Box<dyn StdError + Send + Sync> = Box::new(UpstreamFailure("socket read timed out"));
    let error = TaxonomyError::Timeout {
        provider: "openai".to_owned(),
        model: Some("gpt-4".to_owned()),
        cause,
    };
    let source = error.source().expect("a timeout keeps its cause");
    assert_eq!(source.to_string(), "socket read timed out");
}

#[test]
fn taxonomy_transport_preserves_its_source() {
    let cause: Box<dyn StdError + Send + Sync> = Box::new(UpstreamFailure("connection reset"));
    let error = TaxonomyError::Transport {
        provider: "anthropic".to_owned(),
        model: None,
        cause,
    };
    let source = error.source().expect("a transport failure keeps its cause");
    assert_eq!(source.to_string(), "connection reset");
}

#[test]
fn taxonomy_parse_preserves_its_source() {
    let cause: Box<dyn StdError + Send + Sync> =
        Box::new(UpstreamFailure("unexpected end of JSON"));
    let error = TaxonomyError::Parse {
        provider: "azure".to_owned(),
        model: None,
        cause,
    };
    let source = error.source().expect("a parse failure keeps its cause");
    assert_eq!(source.to_string(), "unexpected end of JSON");
}

#[test]
fn taxonomy_client_message_never_includes_the_cause_text() {
    let secret = "super secret upstream stack trace";
    let cause: Box<dyn StdError + Send + Sync> = Box::new(UpstreamFailure(secret));
    let error = TaxonomyError::Transport {
        provider: "openai".to_owned(),
        model: None,
        cause,
    };
    let class = error.classify();
    let ErrorClass::ApiConnection(context) = &class else {
        panic!("Transport classifies as ApiConnection");
    };
    assert!(!context.message.contains(secret));
    assert!(!error.to_string().contains(secret));
}
