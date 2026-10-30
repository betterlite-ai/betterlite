#![cfg(test)]

use super::*;

#[rstest]
#[case::missing("{}")]
#[case::null("{\"message\":null,\"error\":null,\"code\":null}")]
#[case::text("{\"message\":\"failed\",\"code\":\"code\"}")]
fn error_facts_behavior_tests_behavior(#[case] source: &str) {
    let parsed = parse_provider_error(source.as_bytes()).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), source);
}
