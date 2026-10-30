#![cfg(test)]

use super::*;

#[rstest]
#[case::server_class()]
fn error_facts_behavior_tests_behavior() {
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
