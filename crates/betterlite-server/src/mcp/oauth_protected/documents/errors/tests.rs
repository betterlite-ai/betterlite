#![cfg(test)]

use axum::http::StatusCode;

use super::{not_oauth_protected, upstream_failure};

#[test]
fn the_502_detail_quotes_a_plain_name_with_single_quotes() {
    let failure = upstream_failure(
        "upstream",
        "Upstream oauth-protected-resource metadata unavailable",
    );
    assert_eq!(failure.status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        failure.detail,
        "Upstream oauth-protected-resource metadata unavailable for MCP server 'upstream'"
    );
}

#[test]
fn the_502_detail_uses_double_quotes_for_a_name_with_an_apostrophe() {
    let failure = upstream_failure(
        "it's",
        "Failed to fetch upstream oauth-protected-resource metadata",
    );
    assert_eq!(
        failure.detail,
        "Failed to fetch upstream oauth-protected-resource metadata for MCP server \"it's\""
    );
}

#[test]
fn the_502_detail_escapes_a_control_character_like_python() {
    let failure = upstream_failure(
        "a\u{1}b",
        "Upstream oauth-protected-resource metadata unavailable",
    );
    assert_eq!(
        failure.detail,
        "Upstream oauth-protected-resource metadata unavailable for MCP server 'a\\x01b'"
    );
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
