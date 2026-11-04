#![cfg(test)]

use std::fmt::Write as _;

use super::redaction_tag;
use rstest::rstest;

#[rstest]
#[case::pinned_unicode_decimal_digits(include_str!("fixtures/digits.txt"))]
fn pattern_matching_behavior(#[case] digits: &str) {
    let format = digits
        .trim()
        .chars()
        .fold(String::new(), |mut output, digit| {
            write!(&mut output, "{{pattern_name[{digit}]}}").unwrap();
            output
        });
    assert_eq!(
        redaction_tag(&format, "0123456789").unwrap(),
        "0123456789".repeat(68)
    );
}
