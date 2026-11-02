#![cfg(test)]

use rstest::rstest;

use super::str_repr;

#[rstest]
#[case::plain("clean", "'clean'")]
#[case::single_quote("it's", "\"it's\"")]
#[case::both_quotes("it's \"broken\"", "'it\\'s \"broken\"'")]
#[case::backslash("a\\b", "'a\\\\b'")]
#[case::newline("a\nb", "'a\\nb'")]
#[case::tab("a\tb", "'a\\tb'")]
#[case::carriage_return("a\rb", "'a\\rb'")]
#[case::control_bel("\u{0007}", "'\\x07'")]
#[case::format_zero_width_space("\u{200b}", "'\\u200b'")]
#[case::private_use("\u{f0001}", "'\\U000f0001'")]
#[case::unassigned_noncharacter("\u{1fffe}", "'\\U0001fffe'")]
fn matches_python_repr(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(str_repr(text), expected);
}
