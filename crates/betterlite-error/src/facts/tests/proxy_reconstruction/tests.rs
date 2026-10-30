#![cfg(test)]

use super::*;

#[rstest]
#[case::proxy_reconstruction()]
fn error_facts_behavior_tests_behavior() {
    let facts = ProviderErrorFacts {
        status_code: Some(502),
        headers: Some(BTreeMap::from([(
            String::from("retry-after"),
            String::from("1"),
        )])),
        response: Some(ResponseFacts {
            status_code: 502,
            request: None,
            headers: BTreeMap::new(),
        }),
        status_code_is_synthesized: false,
        message: None,
        body: None,
        litellm_response_headers: None,
        request: None,
    };
    let response = litellm_proxy_response(&facts, "litellm_proxy");
    assert_eq!(
        response.and_then(|value| value.headers.get("retry-after").cloned()),
        Some(String::from("1"))
    );
    let text = "a".repeat(16 * 1024 + 1);
    assert_eq!(bounded_text(&text).len(), 16 * 1024);
}
