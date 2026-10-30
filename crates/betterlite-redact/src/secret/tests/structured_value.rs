use rstest::rstest;

use crate::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

#[rstest]
#[case::value_alone_matches_a_secret_pattern(
    Some("note"),
    "Bearer abcdefghij1234567890",
    "REDACTED"
)]
#[case::value_alone_matches_with_no_key(None, "Bearer abcdefghij1234567890", "REDACTED")]
#[case::key_name_triggers_redaction_even_for_a_bare_value(
    Some("password"),
    "hunter2hunter2",
    "REDACTED"
)]
#[case::non_secret_key_and_value_are_kept(Some("city"), "Springfield", "Springfield")]
#[case::non_secret_value_with_no_key_is_kept(None, "Springfield", "Springfield")]
fn secret_redaction_redaction_tests_behavior(
    #[case] key: Option<&str>,
    #[case] value: &str,
    #[case] expected: &str,
) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    assert_eq!(redactor.redact_structured_value(key, value), expected);
}
