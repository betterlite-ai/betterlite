#![cfg(test)]

use crate::facts::presence::Field;
use rstest::rstest;

use crate::facts::{ProviderErrorBody, RequestFacts};

use super::*;

#[rstest]
#[case::unauthorized(401, StatusFamily::Authentication, 401)]
#[case::forbidden(403, StatusFamily::PermissionDenied, 403)]
#[case::missing(404, StatusFamily::NotFound, 404)]
#[case::timeout(408, StatusFamily::Timeout, 408)]
#[case::limited(429, StatusFamily::RateLimit, 429)]
#[case::server(500, StatusFamily::InternalServer, 500)]
#[case::bad_gateway(502, StatusFamily::BadGateway, 502)]
#[case::unavailable(503, StatusFamily::ServiceUnavailable, 503)]
#[case::bad_request(400, StatusFamily::BadRequest, 400)]
#[case::unknown_client(422, StatusFamily::BadRequest, 400)]
#[case::unknown_server(599, StatusFamily::Api, 599)]
fn error_facts_behavior_tests_behavior(
    #[case] status: u16,
    #[case] expected: StatusFamily,
    #[case] expected_wire_status: u16,
) {
    let error = ProviderErrorFacts {
        status_code: Some(status),
        response: matches!(status, 502 | 503).then(|| ResponseFacts {
            status_code: status,
            headers: BTreeMap::new(),
            request: None,
        }),
        body: match status {
            502 | 503 => Some(ProviderErrorBody {
                message: Field::Missing,
                error: Field::Missing,
                code: Field::Missing,
            }),
            _ => None,
        },
        status_code_is_synthesized: false,
        message: None,
        headers: None,
        litellm_response_headers: None,
        request: None,
    };
    let mapping = map_exception_by_status(&error, "failed", "Provider");
    assert_eq!(mapping.as_ref().map(|value| value.family), Some(expected));
    assert_eq!(
        mapping.map(|value| value.response_present),
        Some(matches!(status, 502 | 503))
    );
    let mapped = status_error_class(&error, "failed", "Provider", None, None).unwrap();
    assert_eq!(mapped.metadata().status_code, expected_wire_status);
}

#[test]
fn error_facts_behavior_tests_behavior_synthesized_status_is_ignored() {
    let error = ProviderErrorFacts {
        status_code: Some(500),
        status_code_is_synthesized: true,
        message: None,
        body: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    };
    assert_eq!(map_exception_by_status(&error, "failed", "Provider"), None);
}

#[test]
fn error_facts_behavior_tests_behavior_status_below_http_errors_is_ignored() {
    let error = ProviderErrorFacts {
        status_code: Some(399),
        status_code_is_synthesized: false,
        message: None,
        body: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    };
    assert_eq!(map_exception_by_status(&error, "failed", "Provider"), None);
}

#[test]
fn error_facts_behavior_tests_behavior_status_class_maps_server_error() {
    let error = ProviderErrorFacts {
        status_code: Some(502),
        status_code_is_synthesized: false,
        message: None,
        body: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    };
    assert!(matches!(
        status_error_class(&error, "failed", "Provider", None, None),
        Some(ErrorClass::BadGateway(_))
    ));
}

#[test]
fn error_facts_behavior_tests_behavior_stub_uses_requested_status() {
    let error = ProviderErrorFacts::default();
    assert_eq!(response_or_stub(&error, 403).status_code, 403);
}

#[test]
fn stub_has_the_pinned_request() {
    let error = ProviderErrorFacts::default();
    let stub = response_or_stub(&error, 403);
    assert_eq!(
        stub.request,
        Some(RequestFacts {
            method: "POST".to_owned(),
            url: "https://docs.litellm.ai/docs".to_owned(),
        })
    );
}
