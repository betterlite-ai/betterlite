#![cfg(test)]
#![cfg(test)]

use rstest::rstest;

use super::*;

#[rstest]
#[case::unauthorized(401, StatusFamily::Authentication)]
#[case::forbidden(403, StatusFamily::PermissionDenied)]
#[case::missing(404, StatusFamily::NotFound)]
#[case::timeout(408, StatusFamily::Timeout)]
#[case::limited(429, StatusFamily::RateLimit)]
#[case::server(500, StatusFamily::InternalServer)]
#[case::unknown_client(422, StatusFamily::BadRequest)]
#[case::unknown_server(599, StatusFamily::Api)]
fn error_facts_behavior_tests_behavior(#[case] status: u16, #[case] expected: StatusFamily) {
    let error = ProviderErrorFacts {
        status_code: Some(status),
        ..ProviderErrorFacts::empty()
    };
    assert_eq!(
        map_exception_by_status(&error, "failed", "Provider").map(|mapping| mapping.family),
        Some(expected)
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
fn error_facts_behavior_tests_behavior_stub_uses_requested_status() {
    let error = ProviderErrorFacts::default();
    assert_eq!(response_or_stub(&error, 403).status_code, 403);
}
