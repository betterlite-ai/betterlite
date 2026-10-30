#![cfg(test)]

use super::*;

#[rstest]
#[case::proxy_empty_headers()]
fn error_facts_behavior_tests_behavior() {
    let facts = ProviderErrorFacts {
        status_code: Some(502),
        response: Some(ResponseFacts {
            status_code: 502,
            headers: BTreeMap::new(),
            request: None,
        }),
        status_code_is_synthesized: false,
        message: None,
        body: None,
        headers: None,
        litellm_response_headers: None,
        request: None,
    };
    assert_eq!(
        litellm_proxy_response(&facts, "openai").map(|response| response.status_code),
        Some(502)
    );
}
