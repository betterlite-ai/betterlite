#![cfg(test)]

use rstest::rstest;

use super::normalize_escaped_newlines;

#[rstest]
#[case::escaped_newline(r"a\nb", "a\nb")]
#[case::escaped_crlf_is_one_newline(r"a\r\nb", "a\nb")]
#[case::escaped_carriage_return(r"a\rb", "a\nb")]
#[case::escaped_fence(r"```python\nprint(1)\n```", "```python\nprint(1)\n```")]
#[case::real_newline_keeps_literal("use \\n\nfor newlines", "use \\n\nfor newlines")]
#[case::real_carriage_return_keeps_literal("a\\nb\rc", "a\\nb\rc")]
#[case::no_escapes("plain text", "plain text")]
#[case::empty("", "")]
fn normalizes_only_fully_escaped_text(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(normalize_escaped_newlines(text), expected);
}
