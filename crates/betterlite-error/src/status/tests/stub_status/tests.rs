#![cfg(test)]

use super::*;

#[rstest]
#[case::stub_status()]
fn error_facts_behavior_tests_behavior() {
    let error = ProviderErrorFacts::default();
    assert_eq!(response_or_stub(&error, 403).status_code, 403);
}
