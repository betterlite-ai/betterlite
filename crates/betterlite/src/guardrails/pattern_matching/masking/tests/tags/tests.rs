#![cfg(test)]

use super::{redaction_tag, rstest};

#[rstest]
#[case::zero_pad("{pattern_name:08}", "email", "EMAIL000")]
#[case::zero_pad_right("{pattern_name:>08}", "email", "000EMAIL")]
#[case::zero_pad_center("{pattern_name:^08}", "email", "0EMAIL00")]
#[case::zero_pad_precision("{pattern_name:08.3}", "email", "EMA00000")]
#[case::zero_pad_index("{pattern_name[0]:03}", "email", "E00")]
#[case::literal_zero_spec("{{pattern_name:08}}", "email", "{pattern_name:08}")]
#[case::multidigit_unicode("{pattern_name[\u{ff11}\u{ff12}]}", "abcdefghijklmn", "M")]
#[case::fullwidth_index("[{pattern_name[\u{ff10}]}]", "email", "[E]")]
#[case::arabic_index("{pattern_name[\u{661}]}", "email", "M")]
#[case::mixed_index("{pattern_name[\u{660}0\u{ff12}]}", "email", "A")]
#[case::mathematical_index("{pattern_name[\u{1d7e3}]}", "email", "M")]
#[case::unicode_nested_index("{pattern_name[\u{ff11}][\u{660}]}", "email", "M")]
#[case::decimal_nine("{pattern_name[\u{19d9}]}", "abcdefghij", "J")]
#[case::unicode_index_padding("[{pattern_name[\u{ff10}]:>3}]", "email", "[  E]")]
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
