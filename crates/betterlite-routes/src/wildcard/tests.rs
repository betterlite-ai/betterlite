#![cfg(test)]

use super::route_matches_wildcard_pattern;

#[test]
fn wildcard_boundary() {
    assert!(route_matches_wildcard_pattern(
        "/containers/",
        "/containers/*"
    ));
    assert!(route_matches_wildcard_pattern(
        "/containers/abc",
        "/containers/*"
    ));
    assert!(!route_matches_wildcard_pattern(
        "/containers",
        "/containers/*"
    ));
}

#[test]
fn final_star_empty_suffix() {
    assert!(route_matches_wildcard_pattern("/foo", "/foo*"));
    assert!(route_matches_wildcard_pattern("/foobar", "/foo*"));
}

#[test]
fn pattern_without_star_matches_only_itself() {
    assert!(route_matches_wildcard_pattern("/foo", "/foo"));
    assert!(!route_matches_wildcard_pattern("/foobar", "/foo"));
}
