#![cfg(test)]

use super::{redaction_tag, rstest};

#[rstest]
#[case::default("[{pattern_name}_REDACTED]", "email", "[EMAIL_REDACTED]")]
#[case::custom("hidden:{pattern_name}", "us_ssn", "hidden:US_SSN")]
#[case::escaped_braces("{{{pattern_name}}}", "ssn", "{SSN}")]
#[case::literal("redacted", "ssn", "redacted")]
#[case::empty("", "ssn", "")]
#[case::unicode("{pattern_name}", "stra\u{df}e", "STRASSE")]
fn pattern_matching_behavior(#[case] format: &str, #[case] name: &str, #[case] expected: &str) {
    assert_eq!(redaction_tag(format, name).unwrap(), expected);
}
