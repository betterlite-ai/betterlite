#![cfg(test)]

use super::*;

#[rstest]
#[case::parser_bounds()]
fn error_facts_behavior_tests_behavior() {
    let oversized = vec![b' '; 64 * 1024 + 1];
    assert_eq!(
        parse_provider_error(&oversized)
            .err()
            .map(|error| error.to_string()),
        Some(String::from("provider error body exceeds the bounded size"))
    );
    let prefix = br#"{"message":"a"}"#;
    let padding = MAX_BODY_BYTES.checked_sub(prefix.len()).unwrap();
    let mut body = Vec::with_capacity(64 * 1024);
    body.extend_from_slice(prefix);
    body.extend(repeat_n(b' ', padding));
    assert_eq!(body.len(), 64 * 1024);
    drop(parse_provider_error(&body).unwrap());
}
