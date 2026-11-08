#![cfg(test)]

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::{
    UpstreamServer, UrlIdentityError, build_upstream_token_request, canonical_resource_uri,
    canonicalize_url_identity, redact_mcp_resource_url, resolve_upstream_resource,
};
use crate::oauth_resource::python_url::{UrlParseError, join, parse};

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn exposed(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

#[rstest]
#[case::default_port("https://h:443/a/b/", "https://h/a/b")]
#[case::userinfo_params_query_fragment("HTTP://User:Pw@H:80/x;y?q=1#f", "http://h/x")]
#[case::params_after_last_slash("https://h/a;b/", "https://h/a;b")]
#[case::params_dropped("https://h/a;b", "https://h/a")]
#[case::unknown_scheme_keeps_params("foo://h/a;b", "foo://h/a;b")]
#[case::ipv6_default_port("https://[::1]:443/mcp/", "https://[::1]/mcp")]
#[case::ipv6_port_kept("https://[::1]:8443/mcp/", "https://[::1]:8443/mcp")]
#[case::relative("/relative/path/", "/relative/path")]
#[case::empty("", "")]
#[case::out_of_range_port("https://h:99999999/x", "https://h:99999999/x")]
#[case::trailing_slashes("https://h/a//", "https://h/a")]
#[case::query_and_fragment_dropped("https://h?x#y", "https://h")]
#[case::no_netloc("mailto:a@b", "mailto:a@b")]
#[case::percent_encoding_kept("https://h/a%20b", "https://h/a%20b")]
fn canonicalize_identity_matches_litellm(#[case] url: &str, #[case] expected: &str) {
    assert_eq!(canonicalize_url_identity(url).unwrap(), expected);
}

#[rstest]
#[case::bracketed_host("https://[abc]/x", UrlParseError::InvalidIpv6)]
#[case::unmatched_bracket("https://[::1/x", UrlParseError::InvalidIpv6)]
fn canonicalize_identity_reports_unparsable_urls(
    #[case] url: &str,
    #[case] expected: UrlParseError,
) {
    assert_eq!(
        canonicalize_url_identity(url).unwrap_err(),
        UrlIdentityError::Parse(expected)
    );
}

#[test]
fn control_characters_stay_in_the_identity() {
    assert_eq!(
        canonicalize_url_identity("https://h/a\tb").unwrap(),
        "https://h/a\tb"
    );
}

#[test]
fn an_ipvfuture_literal_is_rejected() {
    assert_eq!(
        canonicalize_url_identity("https://[v1.foo]/x").unwrap_err(),
        UrlIdentityError::Parse(UrlParseError::InvalidIpv6)
    );
}

#[rstest]
#[case::absolute("https://h:443/a/b/", "https://h/a/b")]
#[case::relative("/relative/path", "")]
#[case::no_host("mailto:a@b", "")]
#[case::empty("", "")]
fn canonical_resource_uri_matches_litellm(#[case] url: &str, #[case] expected: &str) {
    let wanted = (!expected.is_empty()).then_some(expected);
    assert_eq!(canonical_resource_uri(url).unwrap().as_deref(), wanted);
}

#[rstest]
#[case::unset("", "", "")]
#[case::blank("  ", "https://h", "")]
#[case::blank_with_url("", "https://h/mcp", "")]
#[case::auto_without_url("auto", "", "")]
#[case::auto_derives("auto", "https://H:443/mcp/", "https://h/mcp")]
#[case::auto_case_insensitive("AUTO", "https://H:443/mcp/", "https://h/mcp")]
#[case::auto_trimmed(" Auto ", "https://h:443/mcp/", "https://h/mcp")]
#[case::auto_relative("auto", "/relative", "")]
#[case::auto_out_of_range_port("auto", "https://h:99999/x", "https://h:99999/x")]
#[case::explicit_verbatim("  https://exact/Resource  ", "https://h", "https://exact/Resource")]
#[case::auto_without_host(" Auto", "mailto:x@y", "")]
#[case::auto_unparsable("auto", "https://[abc]/x", "")]
fn resolve_upstream_resource_matches_litellm(
    #[case] configured: &str,
    #[case] url: &str,
    #[case] expected: &str,
) {
    let server = UpstreamServer {
        upstream_resource: Some(configured).filter(|value| !value.is_empty()),
        url: (!url.is_empty()).then_some(url),
        server_id: "server-1",
    };
    assert_eq!(
        resolve_upstream_resource(&server).unwrap_or_default(),
        expected
    );
}

#[test]
fn token_request_post_adds_the_resource() {
    let secret_value = secret("sec");
    let server = UpstreamServer {
        upstream_resource: Some("auto"),
        url: Some("https://H:443/mcp/"),
        server_id: "server-1",
    };
    let auth = build_upstream_token_request(
        &server,
        Some("client_secret_post"),
        Some("cid"),
        Some(&secret_value),
    )
    .unwrap();
    assert!(auth.headers.is_empty());
    assert_eq!(exposed(auth.body.get("client_id")), Some("cid"));
    assert_eq!(exposed(auth.body.get("client_secret")), Some("sec"));
    assert_eq!(exposed(auth.body.get("resource")), Some("https://h/mcp"));
}

#[test]
fn token_request_basic_keeps_the_auth_header_and_adds_the_resource() {
    let secret_value = secret("sec");
    let server = UpstreamServer {
        upstream_resource: Some("auto"),
        url: Some("https://H:443/mcp/"),
        server_id: "server-1",
    };
    let auth = build_upstream_token_request(
        &server,
        Some("client_secret_basic"),
        Some("cid"),
        Some(&secret_value),
    )
    .unwrap();
    assert_eq!(
        exposed(auth.headers.get("Authorization")),
        Some("Basic Y2lkOnNlYw==")
    );
    assert_eq!(exposed(auth.body.get("resource")), Some("https://h/mcp"));
}

#[test]
fn token_request_without_a_resource_leaves_the_body_alone() {
    let secret_value = secret("sec");
    let server = UpstreamServer {
        upstream_resource: None,
        url: Some("https://h/mcp"),
        server_id: "server-1",
    };
    let auth = build_upstream_token_request(
        &server,
        Some("client_secret_post"),
        Some("cid"),
        Some(&secret_value),
    )
    .unwrap();
    assert!(!auth.body.contains_key("resource"));
}

#[rstest]
#[case::unset("", "")]
#[case::empty("", "")]
#[case::userinfo_path_query_fragment(
    "https://user:pass@Host.Example:8443/mcp/s/token?x=1#f",
    "https://host.example:8443"
)]
#[case::no_scheme("//HOST:80/x", "//host:80")]
#[case::ipv6_brackets_dropped("https://[::1]:8443/x", "https://::1:8443")]
#[case::ipv6_without_port("https://[::1]/x", "https://::1")]
#[case::port_zero("https://h:0/x", "https://h")]
#[case::out_of_range_port("https://h:99999/x", "")]
#[case::non_numeric_port("https://h:abc/x", "")]
#[case::no_host("https://:8080/x", "")]
#[case::empty_port("https://h:/x", "https://h")]
#[case::bracketed_host("http://[abc]/x", "")]
#[case::not_a_url("not a url", "")]
#[case::padded_port("https://h:0443/x", "https://h:443")]
fn redact_resource_url_matches_litellm(#[case] url: &str, #[case] expected: &str) {
    let wanted = (!expected.is_empty()).then_some(expected);
    let value = (!url.is_empty()).then_some(url);
    assert_eq!(redact_mcp_resource_url(value).as_deref(), wanted);
}

#[test]
fn a_signed_port_is_accepted_when_redacting() {
    assert_eq!(
        redact_mcp_resource_url(Some("https://h:+443/x")).as_deref(),
        Some("https://h:443")
    );
}

#[rstest]
#[case::double_slash_path_without_scheme("", "", "//x", "////x")]
#[case::netloc_scheme_with_rooted_path("http", "", "/x", "http:///x")]
#[case::netloc_scheme_with_empty_path("http", "", "", "http://")]
#[case::netloc_scheme_with_relative_path("http", "", "x", "http:x")]
#[case::other_scheme_keeps_one_slash("mailto", "", "/x", "mailto:/x")]
#[case::other_scheme_empty_path("foo", "", "", "foo:")]
#[case::netloc_adds_the_separator("http", "h", "x", "http://h/x")]
fn join_matches_urlunsplit(
    #[case] scheme: &str,
    #[case] netloc: &str,
    #[case] path: &str,
    #[case] expected: &str,
) {
    assert_eq!(join(scheme, netloc, path, "", ""), expected);
}

#[rstest]
#[case::digit_first("1http://h/")]
#[case::underscore_inside("ht_tp://h/")]
#[case::underscore_after_a_letter("a_b:rest")]
#[case::digit_then_letter("1a:x")]
fn parse_ignores_an_invalid_scheme(#[case] url: &str) {
    let parsed = parse(url).unwrap();
    assert_eq!(parsed.scheme, "");
    assert_eq!(parsed.netloc, "");
    assert_eq!(parsed.path, url);
}
