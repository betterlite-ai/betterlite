#![cfg(test)]

use super::*;

#[rstest]
#[case::stub_request()]
fn error_facts_behavior_tests_behavior() {
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
