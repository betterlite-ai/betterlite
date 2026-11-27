use axum::http::StatusCode;

use super::{not_oauth_protected, python_repr};

#[test]
fn repr_of_a_plain_name_uses_single_quotes() {
    assert_eq!(python_repr("test-oauth-1"), "'test-oauth-1'");
}

#[test]
fn repr_picks_double_quotes_for_a_name_with_an_apostrophe() {
    assert_eq!(python_repr("it's"), "\"it's\"");
}

#[test]
fn repr_escapes_the_quote_it_uses_when_both_appear() {
    assert_eq!(python_repr("a'\"b"), "'a\\'\"b'");
}

#[test]
fn repr_escapes_backslash_and_whitespace_controls() {
    assert_eq!(python_repr("a\\b\nc\rd\te"), "'a\\\\b\\nc\\rd\\te'");
}

#[test]
fn repr_leaves_other_control_characters_raw() {
    assert_eq!(python_repr("a\u{1}b"), "'a\u{1}b'");
}

#[test]
fn the_404_detail_names_the_requested_server() {
    let failure = not_oauth_protected("ghost");
    assert_eq!(failure.status, StatusCode::NOT_FOUND);
    assert_eq!(
        failure.detail,
        "MCP server 'ghost' is not an OAuth-protected resource"
    );
}
