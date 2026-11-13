#![cfg(test)]

use rstest::rstest;

use super::{SanitizationError, sanitize_id};

#[rstest]
#[case::slash("a/b", "contains disallowed characters")]
#[case::backslash("a\\b", "contains disallowed characters")]
#[case::fragment("a#b", "contains disallowed characters")]
#[case::query("a?b", "contains disallowed characters")]
#[case::dot_dot("a..b", "path traversal detected")]
#[case::bare_dot_dot("..", "path traversal detected")]
fn sanitization_rejects_with_the_litellm_message(#[case] identifier: &str, #[case] reason: &str) {
    let error = sanitize_id(identifier).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Invalid identifier '{identifier}': {reason}")
    );
}

#[test]
fn sanitization_checks_characters_before_traversal() {
    let error = sanitize_id("../x").unwrap_err();
    assert_eq!(
        error,
        SanitizationError::DisallowedCharacters {
            identifier: "../x".to_owned()
        }
    );
}

#[rstest]
#[case::plain("UHJvbXB0VmVyc2lvbjox", "UHJvbXB0VmVyc2lvbjox")]
#[case::unreserved("a-b.c_d~e", "a-b.c_d~e")]
#[case::space("a b", "a%20b")]
#[case::percent("100%", "100%25")]
#[case::equals("abc==", "abc%3D%3D")]
#[case::unicode("\u{e9}", "%C3%A9")]
fn sanitization_percent_encodes_everything_but_unreserved(
    #[case] identifier: &str,
    #[case] encoded: &str,
) {
    assert_eq!(sanitize_id(identifier).unwrap(), encoded);
}

#[test]
fn sanitization_encodes_an_already_encoded_slash_instead_of_passing_it() {
    assert_eq!(sanitize_id("a%2Fb").unwrap(), "a%252Fb");
}
