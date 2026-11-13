use std::collections::BTreeMap;

use betterlite_error::error::details::{
    ApiDetails, BudgetExceededDetails, GuardrailDetails, MidStreamFallbackDetails, MidStreamStatus,
    RateLimitDetails, ResponseDetails, SchemaValidationDetails, TimeoutDetails, TimeoutStatus,
};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::cooldown_decisions::failure::FailureClass;

fn error_context() -> ErrorContext {
    ErrorContext {
        message: "probe".to_owned(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

fn response<const DEFAULT: u16>() -> Box<ResponseDetails<DEFAULT>> {
    Box::new(ResponseDetails::from(error_context()))
}

#[rstest]
#[case::authentication(ErrorClass::Authentication(response()), FailureClass::Authentication)]
#[case::not_found(ErrorClass::NotFound(response()), FailureClass::NotFound)]
#[case::permission_denied(
    ErrorClass::PermissionDenied(response()),
    FailureClass::PermissionDenied
)]
#[case::bad_request(ErrorClass::BadRequest(response()), FailureClass::BadRequest)]
#[case::unprocessable_entity(
    ErrorClass::UnprocessableEntity(response()),
    FailureClass::UnprocessableEntity
)]
#[case::timeout(
    ErrorClass::Timeout(Box::new(TimeoutDetails {
        context: error_context(),
        original_status: TimeoutStatus::default(),
    })),
    FailureClass::Timeout
)]
#[case::rate_limit(
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: error_context(),
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    })),
    FailureClass::RateLimit
)]
#[case::budget_exceeded(
    ErrorClass::BudgetExceeded(Box::new(BudgetExceededDetails {
        current_cost: Decimal::ZERO,
        max_budget: Decimal::ONE,
        message: None,
        provider: String::new(),
        entity_type: None,
        entity_id: None,
        status_code: 422,
    })),
    FailureClass::BudgetExceeded
)]
#[case::context_window_exceeded(
    ErrorClass::ContextWindowExceeded(response()),
    FailureClass::ContextWindowExceeded
)]
#[case::content_policy_violation(
    ErrorClass::ContentPolicyViolation(response()),
    FailureClass::ContentPolicyViolation
)]
#[case::service_unavailable(
    ErrorClass::ServiceUnavailable(Box::new(error_context())),
    FailureClass::ServiceUnavailable
)]
#[case::bad_gateway(
    ErrorClass::BadGateway(Box::new(error_context())),
    FailureClass::BadGateway
)]
#[case::internal_server(
    ErrorClass::InternalServer(Box::new(error_context())),
    FailureClass::InternalServer
)]
#[case::api(
    ErrorClass::Api(Box::new(ApiDetails {
        context: error_context(),
        status: 500,
    })),
    FailureClass::Api
)]
#[case::api_connection(
    ErrorClass::ApiConnection(Box::new(error_context())),
    FailureClass::ApiConnection
)]
#[case::api_response_validation(
    ErrorClass::ApiResponseValidation(Box::new(error_context())),
    FailureClass::ApiResponseValidation
)]
#[case::json_schema_validation(
    ErrorClass::JsonSchemaValidation(Box::new(SchemaValidationDetails {
        model: "model".to_owned(),
        provider: "provider".to_owned(),
        raw_response: String::new(),
        schema: String::new(),
    })),
    FailureClass::JsonSchemaValidation
)]
#[case::unsupported_params(
    ErrorClass::UnsupportedParams(Box::new(error_context())),
    FailureClass::UnsupportedParams
)]
#[case::guardrail_raised(
    ErrorClass::GuardrailRaised(Box::new(GuardrailDetails {
        guardrail_name: None,
        message: "probe".to_owned(),
        status_code: 403,
        blocked_content: true,
    })),
    FailureClass::GuardrailRaised
)]
#[case::mid_stream_fallback(
    ErrorClass::MidStreamFallback(Box::new(MidStreamFallbackDetails {
        context: error_context(),
        original_status: MidStreamStatus::default(),
        generated_content: String::new(),
        is_pre_first_chunk: false,
    })),
    FailureClass::MidStreamFallback
)]
#[case::unclassified(
    ErrorClass::Unclassified(Box::new(error_context())),
    FailureClass::Unclassified
)]
fn cooldown_decisions_error_class_mapping(
    #[case] class: ErrorClass,
    #[case] expected: FailureClass,
) {
    assert_eq!(FailureClass::from(&class), expected);
}
