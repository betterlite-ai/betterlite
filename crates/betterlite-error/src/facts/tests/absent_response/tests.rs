#![cfg(test)]

use super::*;

#[rstest]
#[case::absent_response()]
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
    assert_eq!(litellm_proxy_response(&error, "openai"), None);
}
