#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use rust_decimal::Decimal;

use super::{RetryFamily, RetryPolicy};
use crate::error::ErrorClass;
use crate::error::ErrorContext;
use crate::error::details::{
    ApiDetails, BudgetExceededDetails, GuardrailDetails, MidStreamFallbackDetails, MidStreamStatus,
    RateLimitDetails, ResponseDetails, SchemaValidationDetails, TimeoutDetails, TimeoutStatus,
};
use crate::rate_limit::RateLimitCategory;

fn context(message: &str) -> ErrorContext {
    ErrorContext {
        message: message.to_owned(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

fn response<const DEFAULT: u16>(message: &str) -> Box<ResponseDetails<DEFAULT>> {
    Box::new(ResponseDetails::from(context(message)))
}

fn timeout(message: &str) -> ErrorClass {
    ErrorClass::Timeout(Box::new(TimeoutDetails {
        context: context(message),
        original_status: TimeoutStatus::default(),
    }))
}

fn mid_stream(message: &str) -> ErrorClass {
    ErrorClass::MidStreamFallback(Box::new(MidStreamFallbackDetails {
        context: context(message),
        original_status: MidStreamStatus::default(),
        generated_content: String::new(),
        is_pre_first_chunk: true,
    }))
}

fn api(message: &str) -> ErrorClass {
    ErrorClass::Api(Box::new(ApiDetails {
        context: context(message),
        status: 500,
    }))
}

fn json_schema() -> ErrorClass {
    ErrorClass::JsonSchemaValidation(Box::new(SchemaValidationDetails {
        model: "gpt-4".to_owned(),
        provider: "openai".to_owned(),
        raw_response: "{\"bad\": true}".to_owned(),
        schema: "{\"type\": \"object\"}".to_owned(),
    }))
}

fn every_field() -> RetryPolicy {
    RetryPolicy {
        bad_request: Some(1),
        authentication: Some(2),
        timeout: Some(3),
        rate_limit: Some(4),
        content_policy: Some(5),
        internal_server: Some(6),
        service_unavailable: Some(7),
        not_found: Some(8),
        fallback: Some(9),
    }
}

#[rstest]
#[case::authentication(ErrorClass::Authentication(response("authentication")), Some(2))]
#[case::not_found(ErrorClass::NotFound(response("not found")), Some(9))]
#[case::permission_denied(ErrorClass::PermissionDenied(response("permission")), Some(9))]
#[case::bad_request(ErrorClass::BadRequest(response("bad request")), Some(1))]
#[case::unprocessable_entity(ErrorClass::UnprocessableEntity(response("unprocessable")), Some(9))]
#[case::context_window_inherits_bad_request(
    ErrorClass::ContextWindowExceeded(response("too long")),
    Some(1)
)]
#[case::content_policy(ErrorClass::ContentPolicyViolation(response("policy")), Some(5))]
#[case::guardrail_raised(
    ErrorClass::GuardrailRaised(Box::new(GuardrailDetails {
        guardrail_name: None,
        message: "blocked".to_owned(),
        status_code: 400,
        blocked_content: true,
    })),
    Some(9)
)]
#[case::timeout(timeout("timed out"), Some(3))]
#[case::rate_limit(
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context("slow down"),
        category: RateLimitCategory::LitellmRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    })),
    Some(4)
)]
#[case::budget_exceeded(
    ErrorClass::BudgetExceeded(Box::new(BudgetExceededDetails {
        current_cost: Decimal::new(125, 1),
        max_budget: Decimal::from(10_i32),
        message: None,
        provider: "openai".to_owned(),
        entity_type: None,
        entity_id: None,
        status_code: 422,
    })),
    Some(9)
)]
#[case::service_unavailable(
    ErrorClass::ServiceUnavailable(Box::new(context("unavailable"))),
    Some(7)
)]
#[case::bad_gateway(ErrorClass::BadGateway(Box::new(context("gateway"))), Some(9))]
#[case::internal_server(ErrorClass::InternalServer(Box::new(context("internal"))), Some(6))]
#[case::api(api("api failure"), Some(9))]
#[case::api_connection(ErrorClass::ApiConnection(Box::new(context("connection"))), Some(9))]
#[case::api_response_validation(
    ErrorClass::ApiResponseValidation(Box::new(context("validation"))),
    Some(9)
)]
#[case::json_schema_validation(json_schema(), Some(9))]
#[case::unsupported_params_inherits_bad_request(
    ErrorClass::UnsupportedParams(Box::new(context("unsupported"))),
    Some(1)
)]
#[case::mid_stream_inherits_service_unavailable(mid_stream("fallback"), Some(7))]
#[case::unclassified(ErrorClass::Unclassified(Box::new(context("plain"))), Some(9))]
fn a_class_takes_its_own_policy_count(#[case] class: ErrorClass, #[case] expected: Option<i64>) {
    assert_eq!(
        every_field().retries_for(RetryFamily::from(&class), Some(500)),
        expected
    );
}

#[rstest]
#[case::not_found_class(RetryFamily::None, Some(404), Some(8))]
#[case::bad_request_with_404(RetryFamily::BadRequest, Some(404), Some(8))]
#[case::bad_request_keeps_bad_request(RetryFamily::BadRequest, Some(400), Some(1))]
fn a_404_answer_checks_not_found_first(
    #[case] family: RetryFamily,
    #[case] status: Option<u16>,
    #[case] expected: Option<i64>,
) {
    assert_eq!(every_field().retries_for(family, status), expected);
}

#[test]
fn a_404_answer_without_not_found_walks_the_class_chain() {
    let policy = RetryPolicy {
        bad_request: Some(1),
        authentication: None,
        timeout: None,
        rate_limit: None,
        content_policy: None,
        internal_server: None,
        service_unavailable: None,
        not_found: None,
        fallback: Some(9),
    };
    assert_eq!(
        policy.retries_for(RetryFamily::BadRequest, Some(404)),
        Some(1)
    );
}

#[test]
fn a_content_policy_violation_falls_back_to_bad_request() {
    let policy = RetryPolicy {
        bad_request: Some(1),
        authentication: None,
        timeout: None,
        rate_limit: None,
        content_policy: None,
        internal_server: None,
        service_unavailable: None,
        not_found: None,
        fallback: None,
    };
    assert_eq!(
        policy.retries_for(RetryFamily::ContentPolicyViolation, Some(500)),
        Some(1)
    );
}

#[rstest]
#[case::authentication(RetryFamily::Authentication, Some(500))]
#[case::timeout(RetryFamily::Timeout, Some(500))]
#[case::rate_limit(RetryFamily::RateLimit, Some(404))]
#[case::content_policy(RetryFamily::ContentPolicyViolation, Some(500))]
#[case::bad_request(RetryFamily::BadRequest, Some(404))]
#[case::service_unavailable(RetryFamily::ServiceUnavailable, Some(500))]
#[case::internal_server(RetryFamily::InternalServer, Some(500))]
#[case::no_slot(RetryFamily::None, Some(500))]
fn an_empty_policy_grants_nothing(#[case] family: RetryFamily, #[case] status: Option<u16>) {
    assert_eq!(RetryPolicy::default().retries_for(family, status), None);
}

#[rstest]
#[case::zero(Some(0))]
#[case::negative(Some(-1))]
fn a_not_found_count_is_used_as_given(#[case] not_found: Option<i64>) {
    let policy = RetryPolicy {
        bad_request: None,
        authentication: None,
        timeout: None,
        rate_limit: None,
        content_policy: None,
        internal_server: None,
        service_unavailable: None,
        not_found,
        fallback: Some(9),
    };
    assert_eq!(policy.retries_for(RetryFamily::None, Some(404)), not_found);
}

#[test]
fn the_wire_reads_litellm_retry_policy_field_names() {
    let json = r#"{
        "BadRequestErrorRetries": 1,
        "AuthenticationErrorRetries": 2,
        "TimeoutErrorRetries": 3,
        "RateLimitErrorRetries": 4,
        "ContentPolicyViolationErrorRetries": 5,
        "InternalServerErrorRetries": 6,
        "ServiceUnavailableErrorRetries": 7,
        "NotFoundErrorRetries": 8,
        "DefaultRetries": 9
    }"#;
    let policy: RetryPolicy = serde_json::from_str(json).expect("reads every policy field");
    assert_eq!(policy, every_field());
}

#[test]
fn an_unknown_retry_policy_field_is_rejected() {
    let error = serde_json::from_str::<RetryPolicy>(r#"{"UnknownErrorRetries": 1}"#).unwrap_err();
    assert!(error.to_string().contains("UnknownErrorRetries"));
}
