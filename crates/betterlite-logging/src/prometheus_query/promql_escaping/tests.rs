#![cfg(test)]

use super::quote_promql_string_literal;
use rstest::rstest;

#[rstest]
#[case::plain("sk-abc123", r#""sk-abc123""#)]
#[case::empty("", r#""""#)]
#[case::double_quote(r#"a"b"#, r#""a\"b""#)]
#[case::backslash(r"a\b", r#""a\\b""#)]
#[case::backslash_before_quote(r#"a\""#, r#""a\\\"""#)]
#[case::newline("a\nb", r#""a\nb""#)]
#[case::carriage_return("a\rb", r#""a\rb""#)]
#[case::tab("a\tb", r#""a\tb""#)]
#[case::backspace("a\u{8}b", r#""a\bb""#)]
#[case::form_feed("a\u{c}b", r#""a\fb""#)]
#[case::nul("a\u{0}b", r#""a\u0000b""#)]
#[case::escape_character("a\u{1b}b", r#""a\u001bb""#)]
#[case::unit_separator("a\u{1f}b", r#""a\u001fb""#)]
#[case::space_is_literal("a b", r#""a b""#)]
#[case::delete_is_literal("a\u{7f}b", "\"a\u{7f}b\"")]
#[case::non_ascii_is_literal(
    "cl\u{e9}-\u{43a}\u{43b}\u{44e}\u{447}-\u{9375}-\u{1f511}",
    "\"cl\u{e9}-\u{43a}\u{43b}\u{44e}\u{447}-\u{9375}-\u{1f511}\""
)]
#[case::injection_attempt(
    r#"x"} or vector(1) or foo{a="b"#,
    r#""x\"} or vector(1) or foo{a=\"b""#
)]
fn quotes_a_value_as_a_promql_string_literal(#[case] value: &str, #[case] literal: &str) {
    assert_eq!(quote_promql_string_literal(value), literal);
}

#[rstest]
#[case::quote_and_backslash(r#"a"\b"#)]
#[case::controls("\u{0}\u{1}\u{8}\t\n\u{b}\u{c}\r\u{1f}")]
#[case::unicode("\u{65e5}\u{672c}\u{8a9e} \u{1f511}")]
fn matches_json_string_quoting(#[case] value: &str) {
    assert_eq!(
        quote_promql_string_literal(value),
        serde_json::to_string(value).unwrap()
    );
}
