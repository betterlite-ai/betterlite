#![cfg(test)]

use std::{fs, path::PathBuf, str::from_utf8};

use rstest::rstest;

use super::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

enum Operation {
    String,
    InternalDetails,
}

#[rstest]
#[case::redact_string_delegates_to_the_secret_pattern(
    Operation::String,
    "Bearer abcdefghij1234567890",
    "REDACTED"
)]
#[case::redact_internal_details_drops_the_traceback(
    Operation::InternalDetails,
    "boom\nTraceback (most recent call last):\n  oops",
    "boom"
)]
fn secret_redaction_redaction_tests_behavior(
    #[case] operation: Operation,
    #[case] input: &str,
    #[case] expected: &str,
) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let output = match operation {
        Operation::String => redactor.redact_string(input),
        Operation::InternalDetails => redactor.redact_internal_details(input),
    };
    assert_eq!(output, expected);
}

#[test]
fn minimum_custom_key_length_raises_or_lowers_the_shortest_accepted_sk_key() {
    let short_key = "sk-abcdefghijklmno";
    let default_redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    assert_eq!(default_redactor.redact_string(short_key), "REDACTED");
    let stricter_redactor = Redactor::new(20).unwrap();
    assert_eq!(stricter_redactor.redact_string(short_key), short_key);
}

#[test]
fn redact_structured_value_delegates_through_the_redactor() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    assert_eq!(
        redactor.redact_structured_value(Some("password"), "hunter2hunter2"),
        "REDACTED"
    );
    assert_eq!(
        redactor.redact_structured_value(Some("city"), "Springfield"),
        "Springfield"
    );
}

mod structured_value;

#[rstest]
fn replays_the_redaction_corpus(#[files("fuzz/corpus/redaction/*")] path: PathBuf) {
    let Ok(bytes) = fs::read(&path) else {
        panic!("{}", path.display());
    };
    let Ok(text) = from_utf8(&bytes) else {
        return;
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    drop(redactor.redact_string(text));
    drop(redactor.redact_internal_details(text));
    drop(redactor.redact_structured_value(Some("password"), text));
    drop(redactor.redact_structured_value(None, text));
}
