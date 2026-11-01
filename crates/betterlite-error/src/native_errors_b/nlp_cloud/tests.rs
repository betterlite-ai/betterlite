#![cfg(test)]

use rstest::rstest;

use super::*;

fn input(error_str: &str, status_code: Option<u16>) -> NlpCloudExceptionInput {
    NlpCloudExceptionInput {
        model: "nlp_cloud/some-model".to_owned(),
        error_str: error_str.to_owned(),
        status_code,
        message: None,
    }
}

fn family(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::ContextWindowExceeded(_) => "context_window_exceeded",
        ErrorClass::BadRequest(_) => "bad_request",
        ErrorClass::Authentication(_) => "authentication",
        ErrorClass::Timeout(_) => "timeout",
        ErrorClass::RateLimit(_) => "rate_limit",
        ErrorClass::Api(_) => "api",
        ErrorClass::ServiceUnavailable(_) => "service_unavailable",
        ErrorClass::NotFound(_)
        | ErrorClass::PermissionDenied(_)
        | ErrorClass::UnprocessableEntity(_)
        | ErrorClass::BudgetExceeded(_)
        | ErrorClass::ContentPolicyViolation(_)
        | ErrorClass::BadGateway(_)
        | ErrorClass::InternalServer(_)
        | ErrorClass::ApiConnection(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => {
            panic!("nlp_cloud classification never produces {class:?}")
        }
    }
}

#[rstest]
// "detail" in error_str, line 1635-1657
#[case::detail_context_window(
    input("detail: Input text length should not exceed 512", None),
    Some("context_window_exceeded")
)]
#[case::detail_bad_request(
    input("detail: value is not a valid integer", None),
    Some("bad_request")
)]
#[case::detail_generic_api(input("detail: something else entirely", None), Some("api"))]
// hasattr(status_code), line 1658-1713
#[case::status_400(input("boom", Some(400)), Some("bad_request"))]
#[case::status_406(input("boom", Some(406)), Some("bad_request"))]
#[case::status_413(input("boom", Some(413)), Some("bad_request"))]
#[case::status_422(input("boom", Some(422)), Some("bad_request"))]
#[case::status_401(input("boom", Some(401)), Some("authentication"))]
#[case::status_403(input("boom", Some(403)), Some("authentication"))]
#[case::status_522(input("boom", Some(522)), Some("timeout"))]
#[case::status_524(input("boom", Some(524)), Some("timeout"))]
#[case::status_429(input("boom", Some(429)), Some("rate_limit"))]
#[case::status_402(input("boom", Some(402)), Some("rate_limit"))]
#[case::status_500(input("boom", Some(500)), Some("api"))]
#[case::status_503(input("boom", Some(503)), Some("api"))]
#[case::status_504(input("boom", Some(504)), Some("service_unavailable"))]
#[case::status_520(input("boom", Some(520)), Some("service_unavailable"))]
#[case::status_other_falls_to_generic_api(input("boom", Some(418)), Some("api"))]
#[case::no_detail_no_status_falls_through(input("boom", None), None)]
fn native_errors_b_behavior_tests_behavior(
    #[case] input: NlpCloudExceptionInput,
    #[case] expected_family: Option<&str>,
) {
    let mapped = map_nlp_cloud_exception(&input);
    match expected_family {
        None => assert!(
            mapped.is_none(),
            "expected no classification, got {mapped:?}"
        ),
        Some(expected) => {
            let class = mapped.expect("a matching predicate must produce a class");
            assert_eq!(family(&class), expected);
        }
    }
}

#[test]
fn detail_predicate_wins_over_a_competing_status_code() {
    // line 1635 gates everything below it; a status_code that would otherwise authenticate never runs
    let class = map_nlp_cloud_exception(&input(
        "detail: Input text length should not exceed 512",
        Some(401),
    ));
    assert_eq!(
        family(&class.expect("detail always raises")),
        "context_window_exceeded"
    );
}

#[test]
fn status_500_and_503_carry_through_their_own_status_not_a_shared_default() {
    let five_hundred = map_nlp_cloud_exception(&input("boom", Some(500)));
    let ErrorClass::Api(details) = five_hundred.expect("status 500 always raises") else {
        panic!("expected Api");
    };
    assert_eq!(details.status, 500);

    let five_oh_three = map_nlp_cloud_exception(&input("boom", Some(503)));
    let ErrorClass::Api(service_details) = five_oh_three.expect("status 503 always raises") else {
        panic!("expected Api");
    };
    assert_eq!(service_details.status, 503);
}

#[test]
fn missing_message_falls_back_to_error_str_instead_of_panicking() {
    // every status branch reads original_exception.message unconditionally and crashes without it in LiteLLM; betterlite falls back to error_str instead (docs/deviations/B242.yaml)
    let class = map_nlp_cloud_exception(&input("boom", Some(401)));
    let ErrorClass::Authentication(details) = class.expect("status 401 always raises") else {
        panic!("expected Authentication");
    };
    assert_eq!(details.message, "nlp_cloud: boom");
}

#[test]
fn timeout_default_ignores_the_original_522_and_524_status() {
    // Timeout() never receives exception_status_code here, so it defaults to 408 regardless of the HTTP status
    let class = map_nlp_cloud_exception(&input("boom", Some(522)));
    let ErrorClass::Timeout(details) = class.expect("status 522 always raises") else {
        panic!("expected Timeout");
    };
    assert_eq!(details.original_status.0, 408);
}
