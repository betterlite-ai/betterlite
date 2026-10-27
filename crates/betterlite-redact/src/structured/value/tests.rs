#![cfg(test)]

use super::SensitiveDataMasker;
use rstest::rstest;

#[rstest]
#[case::empty("", 4, 4, true, "")]
#[case::short("short", 4, 4, true, "*****")]
#[case::threshold("12345678", 4, 4, true, "********")]
#[case::partial("123456789", 4, 4, true, "1234*6789")]
#[case::no_suffix("123456789", 4, 0, true, "1234*****")]
#[case::no_prefix("123456789", 0, 4, true, "*****6789")]
#[case::entire("123", 0, 0, true, "***")]
#[case::short_visible("short", 4, 4, false, "short")]
#[case::unicode(
    "\u{3b1}\u{3b2}\u{3b3}\u{3b4}\u{3b5}\u{3b6}\u{3b7}\u{3b8}\u{3b9}",
    4,
    4,
    true,
    "\u{3b1}\u{3b2}\u{3b3}\u{3b4}*\u{3b6}\u{3b7}\u{3b8}\u{3b9}"
)]
fn structured_redaction_behavior_tests_behavior(
    #[case] input: &str,
    #[case] prefix: usize,
    #[case] suffix: usize,
    #[case] short: bool,
    #[case] expected: &str,
) {
    let mut masker = SensitiveDataMasker::new();
    masker.visible_prefix = prefix;
    masker.visible_suffix = suffix;
    masker.mask_short_values = short;
    assert_eq!(masker.mask_value(input), expected);
}

#[rstest]
#[case::input_over("abcdef", 5, "*", "REDACTED")]
#[case::input_equal("abcde", 5, "*", "*****")]
#[case::unicode_expansion("ab", 5, "\u{1f980}", "REDACTED")]
#[case::mask_equal("ab", 4, "**", "****")]
#[case::output_equal("abcdefghi", 9, "*", "abcd*fghi")]
#[case::output_over("abcdefghi", 9, "**", "REDACTED")]
fn mask_value_bounds(
    #[case] input: &str,
    #[case] work: usize,
    #[case] mask: &str,
    #[case] expected: &str,
) {
    let mut masker = SensitiveDataMasker::new();
    masker.mask_char = mask.to_owned();
    masker.max_work = work;
    assert_eq!(masker.mask_value(input), expected);
}
