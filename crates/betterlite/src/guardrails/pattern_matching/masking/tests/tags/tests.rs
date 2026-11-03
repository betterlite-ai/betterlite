#![cfg(test)]

use super::{redaction_tag, rstest};

#[rstest]
#[case::indexed_first("[{pattern_name[0]}]", "email", "[E]")]
#[case::indexed_last("{pattern_name[4]}", "email", "L")]
#[case::indexed_unicode("{pattern_name[1]}", "\u{e9}mail", "M")]
#[case::indexed_expansion("{pattern_name[1]}", "\u{df}x", "S")]
#[case::indexed_nested("{pattern_name[1][0]}", "email", "M")]
#[case::indexed_padding("[{pattern_name[0]:>3}]", "email", "[  E]")]
#[case::default("[{pattern_name}_REDACTED]", "email", "[EMAIL_REDACTED]")]
#[case::custom("hidden:{pattern_name}", "us_ssn", "hidden:US_SSN")]
#[case::escaped_braces("{{{pattern_name}}}", "ssn", "{SSN}")]
#[case::literal("redacted", "ssn", "redacted")]
#[case::empty("", "ssn", "")]
#[case::unicode("{pattern_name}", "stra\u{df}e", "STRASSE")]
fn pattern_matching_behavior(#[case] format: &str, #[case] name: &str, #[case] expected: &str) {
    assert_eq!(redaction_tag(format, name).unwrap(), expected);
}
