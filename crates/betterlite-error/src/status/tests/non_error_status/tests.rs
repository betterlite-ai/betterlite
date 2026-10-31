#![cfg(test)]

use super::*;

#[rstest]
#[case::non_error_status()]
fn error_facts_behavior_tests_behavior() {
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
