use rstest::rstest;

use crate::openapi_tools::path_values::{PathValueError, sanitize_path_value};

use super::support::json;

#[rstest]
#[case::plain("abc", "abc")]
#[case::letters_digits_and_safe_characters("aZ09_.-~", "aZ09_.-~")]
#[case::space("a b", "a%20b")]
#[case::question_mark("a?b", "a%3Fb")]
#[case::ampersand_and_equals("a&b=c", "a%26b%3Dc")]
#[case::percent("100%", "100%25")]
#[case::hash("a#b", "a%23b")]
#[case::plus("a+b", "a%2Bb")]
#[case::apostrophe("a'b", "a%27b")]
#[case::parentheses("(a)", "%28a%29")]
#[case::non_ascii("ü", "%C3%BC")]
#[case::single_dot(".", ".")]
#[case::three_dots("...", "...")]
#[case::dot_dot_inside_a_word("a..b", "a..b")]
fn percent_encodes_like_python_quote(#[case] raw: &str, #[case] expected: &str) {
    assert_eq!(
        sanitize_path_value(&json(&format!("{raw:?}")), "id").unwrap(),
        expected
    );
}

#[test]
fn an_empty_text_value_stays_empty() {
    assert_eq!(sanitize_path_value(&json(r#""""#), "id").unwrap(), "");
}

#[test]
fn null_becomes_empty() {
    assert_eq!(sanitize_path_value(&json("null"), "id").unwrap(), "");
}

#[rstest]
#[case::true_is_capitalized("true", "True")]
#[case::false_is_capitalized("false", "False")]
#[case::integer("42", "42")]
#[case::decimal("1.5", "1.5")]
fn scalars_use_python_str_text(#[case] raw: &str, #[case] expected: &str) {
    assert_eq!(sanitize_path_value(&json(raw), "id").unwrap(), expected);
}

#[test]
fn a_container_goes_out_as_its_json_text() {
    assert_eq!(
        sanitize_path_value(&json(r#"[1,"a"]"#), "id").unwrap(),
        "%5B1%2C%22a%22%5D"
    );
}

#[rstest]
#[case::forward_slash("a/b")]
#[case::a_leading_slash("/a")]
#[case::a_trailing_slash("a/")]
#[case::a_backslash_becomes_a_separator("a\\b")]
#[case::a_backslash_before_dot_dot("\\..")]
fn rejects_path_separators(#[case] raw: &str) {
    assert_eq!(
        sanitize_path_value(&json(&format!("{raw:?}")), "id"),
        Err(PathValueError::Separator {
            name: "id".to_owned()
        })
    );
}

#[test]
fn rejects_a_dot_dot_value() {
    let error = sanitize_path_value(&json(r#""..""#), "repo").unwrap_err();
    assert_eq!(
        error,
        PathValueError::DotSegment {
            name: "repo".to_owned()
        }
    );
    assert_eq!(
        error.to_string(),
        "Path parameter 'repo' cannot include '.' or '..' segments"
    );
}

#[test]
fn the_separator_error_names_the_parameter() {
    let error = sanitize_path_value(&json(r#""a/b""#), "owner").unwrap_err();
    assert_eq!(
        error.to_string(),
        "Path parameter 'owner' must not contain path separators"
    );
}
