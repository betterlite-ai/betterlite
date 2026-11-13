#![cfg(test)]

use rstest::rstest;

use super::UnappliableRequestRewrite;

#[rstest]
#[case::named(Some("pii-masker"), "pii-masker")]
#[case::unnamed(None, "unknown")]
#[case::empty_name_is_kept(Some(""), "")]
fn the_error_names_the_guardrail(#[case] name: Option<&str>, #[case] expected: &str) {
    assert_eq!(
        UnappliableRequestRewrite::new(name).guardrail_name,
        expected
    );
}

#[test]
fn the_message_says_the_request_was_rejected() {
    let error = UnappliableRequestRewrite::new(Some("pii-masker"));
    assert_eq!(
        error.to_string(),
        "Guardrail 'pii-masker' rewrote the request in a way this endpoint cannot apply, so the request was rejected rather than sent unrewritten"
    );
}
