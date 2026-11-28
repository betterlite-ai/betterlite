#![cfg(test)]

use super::redirect_location;

const CALLBACK: &str = "http://127.0.0.1:1456/callback";

#[test]
fn a_plain_uri_gets_a_query_string() {
    assert_eq!(
        redirect_location(CALLBACK, "code-1", "state-1"),
        "http://127.0.0.1:1456/callback?code=code-1&state=state-1"
    );
}

#[test]
fn a_uri_with_a_query_gets_an_ampersand() {
    assert_eq!(
        redirect_location("http://127.0.0.1:1456/callback?tenant=acme", "code-1", "s"),
        "http://127.0.0.1:1456/callback?tenant=acme&code=code-1&state=s"
    );
}

#[test]
fn a_space_in_the_state_becomes_a_plus() {
    assert_eq!(
        redirect_location(CALLBACK, "code-1", "a b"),
        "http://127.0.0.1:1456/callback?code=code-1&state=a+b"
    );
}

#[test]
fn a_slash_and_ampersand_in_the_state_are_percent_encoded() {
    assert_eq!(
        redirect_location(CALLBACK, "code-1", "a/b&c=d"),
        "http://127.0.0.1:1456/callback?code=code-1&state=a%2Fb%26c%3Dd"
    );
}

#[test]
fn a_percent_in_the_state_is_encoded_once() {
    assert_eq!(
        redirect_location(CALLBACK, "code-1", "100%"),
        "http://127.0.0.1:1456/callback?code=code-1&state=100%25"
    );
}

#[test]
fn a_unicode_state_is_utf8_percent_encoded() {
    assert_eq!(
        redirect_location(CALLBACK, "code-1", "\u{e9}"),
        "http://127.0.0.1:1456/callback?code=code-1&state=%C3%A9"
    );
}

#[test]
fn the_unreserved_marks_in_the_state_are_kept() {
    assert_eq!(
        redirect_location(CALLBACK, "code-1", "a_-._~z"),
        "http://127.0.0.1:1456/callback?code=code-1&state=a_-._~z"
    );
}

#[test]
fn the_uri_keeps_starlettes_safe_marks() {
    assert_eq!(
        redirect_location("http://127.0.0.1:1/cb(a)[b]!$@,;", "c", "s"),
        "http://127.0.0.1:1/cb(a)[b]!$@,;?code=c&state=s"
    );
}

#[test]
fn a_quote_in_the_uri_is_encoded() {
    assert_eq!(
        redirect_location("http://127.0.0.1:1/cb?a=\"b\"", "c", "s"),
        "http://127.0.0.1:1/cb?a=%22b%22&code=c&state=s"
    );
}

#[test]
fn a_hash_in_the_uri_stays_a_fragment_marker() {
    assert_eq!(
        redirect_location("http://127.0.0.1:1/cb#frag", "c", "s"),
        "http://127.0.0.1:1/cb#frag?code=c&state=s"
    );
}
