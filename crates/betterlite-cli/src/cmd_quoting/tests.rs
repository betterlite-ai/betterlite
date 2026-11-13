#![cfg(test)]

use rstest::rstest;

use super::quote_for_cmd;

#[rstest]
#[case::plain("hello", "\"hello\"")]
#[case::spaces("a b", "\"a b\"")]
#[case::empty("", "\"\"")]
#[case::embedded_quote("a\"b", "\"a\"\"b\"")]
#[case::only_quote("\"", "\"\"\"\"")]
#[case::quoted_phrase("a \"b\" c", "\"a \"\"b\"\" c\"")]
#[case::inner_backslash("a\\b", "\"a\\b\"")]
#[case::trailing_backslash("a\\", "\"a\\\\\"")]
#[case::trailing_backslashes("ab\\\\\\", "\"ab\\\\\\\\\\\\\"")]
#[case::backslash_before_quote("a\\\"b", "\"a\\\\\"\"b\"")]
#[case::backslashes_before_a_quote("a\\\\\"b", "\"a\\\\\\\\\"\"b\"")]
#[case::percent("100%", "\"100%%cd:~,%\"")]
#[case::percent_pair("%PATH%", "\"%%cd:~,%PATH%%cd:~,%\"")]
#[case::trailing_backslash_before_percent("a\\%", "\"a\\%%cd:~,%\"")]
fn quote_for_cmd_quotes_the_token(#[case] token: &str, #[case] expected: &str) {
    assert_eq!(quote_for_cmd(token), expected);
}
