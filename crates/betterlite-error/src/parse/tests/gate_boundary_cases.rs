use super::*;

#[rstest]
fn error_facts_behavior_tests_behavior() {
    let prefix = br#"{"error":{"code":429}}"#;
    let mut body = prefix.to_vec();
    body.extend(repeat_n(
        b' ',
        MAX_BODY_BYTES.checked_sub(prefix.len()).unwrap(),
    ));
    assert_eq!(body.len(), MAX_BODY_BYTES);
    assert_eq!(get_body_error_code(from_utf8(&body).unwrap()), Some(429));
    body.push(b' ');
    assert_eq!(get_body_error_code(from_utf8(&body).unwrap()), None);
}
