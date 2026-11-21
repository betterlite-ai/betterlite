#![cfg(test)]

use rstest::rstest;

use super::is_latin1_encodable;

#[rstest]
#[case::empty("", true)]
#[case::ascii("key-prod-1", true)]
#[case::latin1_supplement("pri\u{f6}rity", true)]
#[case::last_latin1("\u{ff}", true)]
#[case::first_beyond_latin1("\u{100}", false)]
#[case::cyrillic("\u{43f}\u{440}\u{438}", false)]
#[case::emoji("prio\u{1f680}", false)]
fn latin1_encodability(#[case] value: &str, #[case] expected: bool) {
    assert_eq!(is_latin1_encodable(value), expected);
}
