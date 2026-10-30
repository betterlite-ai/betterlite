#![cfg(test)]

use super::*;

#[rstest]
#[case::body_boundaries()]
fn error_facts_behavior_tests_behavior() {
    let empty = ProviderErrorBody {
        message: Field::Value(String::new()),
        error: Field::Missing,
        code: Field::Missing,
    };
    let overhead = serde_json::to_vec(&empty).unwrap().len();
    let body = ProviderErrorBody {
        message: Field::Value("a".repeat(MAX_BODY_BYTES.checked_sub(overhead).unwrap())),
        error: Field::Missing,
        code: Field::Missing,
    };
    assert_eq!(serde_json::to_vec(&body).unwrap().len(), MAX_BODY_BYTES);
    assert!(bounded_body(&body).is_some());
    let oversized = ProviderErrorBody {
        message: Field::Value(format!(
            "{}a",
            body.message.as_ref().map_or("", String::as_str)
        )),
        error: Field::Missing,
        code: Field::Missing,
    };
    assert!(bounded_body(&oversized).is_none());
}
