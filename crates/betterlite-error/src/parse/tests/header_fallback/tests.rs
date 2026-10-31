#![cfg(test)]

use super::*;

#[rstest]
#[case::header_fallback()]
fn error_facts_behavior_tests_behavior() {
    let facts = ProviderErrorFacts {
        headers: Some(BTreeMap::new()),
        response: Some(ResponseFacts {
            status_code: 502,
            request: None,
            headers: BTreeMap::from([(String::from("a"), String::from("two"))]),
        }),
        litellm_response_headers: Some(BTreeMap::from([(
            String::from("a"),
            String::from("three"),
        )])),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        request: None,
    };
    assert_eq!(
        get_response_headers(&facts).and_then(|headers| headers.get("a").cloned()),
        Some(String::from("two"))
    );
    let only_empty = ProviderErrorFacts {
        headers: Some(BTreeMap::new()),
        response: Some(ResponseFacts {
            status_code: 502,
            request: None,
            headers: BTreeMap::new(),
        }),
        litellm_response_headers: Some(BTreeMap::new()),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        request: None,
    };
    assert_eq!(get_response_headers(&only_empty), Some(BTreeMap::new()));
}
