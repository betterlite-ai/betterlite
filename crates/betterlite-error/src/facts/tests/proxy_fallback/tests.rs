#![cfg(test)]

use super::*;

#[rstest]
#[case::proxy_fallback()]
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
        headers: Some(BTreeMap::from([(
            String::from("retry-after"),
            String::from("1"),
        )])),
        litellm_response_headers: None,
        request: None,
    };
    assert!(
        litellm_proxy_response(&facts, "openai")
            .is_some_and(|response| response.headers.is_empty())
    );
}
