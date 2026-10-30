#![cfg(test)]

use super::*;

#[rstest]
#[case::code_boundary()]
fn error_facts_behavior_tests_behavior() {
    let prefix = br#"{"error":{"code":429}}"#;
    let mut at_limit = prefix.to_vec();
    at_limit.extend(repeat_n(
        b' ',
        MAX_BODY_BYTES.checked_sub(prefix.len()).unwrap(),
    ));
    assert_eq!(at_limit.len(), MAX_BODY_BYTES);
    assert_eq!(
        get_body_error_code(from_utf8(&at_limit).unwrap()),
        Some(429)
    );

    at_limit.push(b' ');
    assert_eq!(get_body_error_code(from_utf8(&at_limit).unwrap()), None);
}
