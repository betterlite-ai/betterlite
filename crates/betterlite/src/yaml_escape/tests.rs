#![cfg(test)]

use rstest::rstest;

use super::yaml_escape;

#[rstest]
#[case::backslash("a\\b", "a\\\\b")]
#[case::double_quote("a\"b", "a\\\"b")]
#[case::newline("a\nb", "a\\nb")]
#[case::carriage_return("a\rb", "a\\rb")]
#[case::tab("a\tb", "a\\tb")]
#[case::plain_text_is_unchanged("a plain value", "a plain value")]
fn escapes_scalars_for_double_quoted_yaml(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(yaml_escape(input), expected);
}

#[test]
fn escapes_every_special_character_in_one_pass() {
    assert_eq!(
        yaml_escape("line1\\line2\"line3\nline4\rline5\tend"),
        "line1\\\\line2\\\"line3\\nline4\\rline5\\tend"
    );
}
