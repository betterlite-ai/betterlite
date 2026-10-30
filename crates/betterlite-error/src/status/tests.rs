#![cfg(test)]
#![cfg(test)]

use rstest::rstest;

use crate::facts::ProviderErrorBody;

use super::*;

#[rstest]
#[case::unauthorized(401, StatusFamily::Authentication)]
#[case::forbidden(403, StatusFamily::PermissionDenied)]
#[case::missing(404, StatusFamily::NotFound)]
#[case::timeout(408, StatusFamily::Timeout)]
#[case::limited(429, StatusFamily::RateLimit)]
#[case::server(500, StatusFamily::InternalServer)]
#[case::bad_gateway(502, StatusFamily::BadGateway)]
#[case::unavailable(503, StatusFamily::ServiceUnavailable)]
#[case::bad_request(400, StatusFamily::BadRequest)]
#[case::unknown_client(422, StatusFamily::BadRequest)]
#[case::unknown_server(599, StatusFamily::Api)]
fn error_facts_behavior_tests_behavior(#[case] status: u16, #[case] expected: StatusFamily) {
    let error = ProviderErrorFacts {
        status_code: Some(status),
        body: match status {
            502 | 503 => Some(ProviderErrorBody {
                message: None,
                error: None,
                code: None,
            }),
            _ => None,
        },
        ..ProviderErrorFacts::empty()
    };
    let mapping = map_exception_by_status(&error, "failed", "Provider");
    assert_eq!(mapping.as_ref().map(|value| value.family), Some(expected));
    assert_eq!(
        mapping.map(|value| value.response_present),
        Some(matches!(status, 502 | 503))
    );
}

#[test]
fn error_facts_behavior_tests_behavior_synthesized_status_is_ignored() {
    let error = ProviderErrorFacts {
        status_code: Some(500),
        status_code_is_synthesized: true,
        ..ProviderErrorFacts::empty()
    };
    assert_eq!(map_exception_by_status(&error, "failed", "Provider"), None);
}

#[test]
fn error_facts_behavior_tests_behavior_status_below_http_errors_is_ignored() {
    let error = ProviderErrorFacts {
        status_code: Some(399),
        ..ProviderErrorFacts::empty()
    };
    assert_eq!(map_exception_by_status(&error, "failed", "Provider"), None);
}

#[test]
fn error_facts_behavior_tests_behavior_status_class_maps_server_error() {
    let error = ProviderErrorFacts {
        status_code: Some(502),
        ..ProviderErrorFacts::empty()
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
