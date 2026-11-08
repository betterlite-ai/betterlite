#![cfg(test)]

use rstest::rstest;

use super::{RequestOrigin, get_request_base_url, normalize_proxy_base_url, strip_default_port};
use crate::oauth_resource::python_url::UrlParseError;

fn optional(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn origin<'request>(
    base_url: &'request str,
    proto: &'request str,
    host: &'request str,
    port: &'request str,
    from_trusted_proxy: bool,
) -> RequestOrigin<'request> {
    RequestOrigin {
        base_url,
        x_forwarded_proto: optional(proto),
        x_forwarded_host: optional(host),
        x_forwarded_port: optional(port),
        from_trusted_proxy,
    }
}

#[rstest]
#[case::https_default("https", "LLM.Example.com:443", "llm.example.com")]
#[case::http_default("http", "h:80", "h")]
#[case::https_non_default("https", "h:8443", "h:8443")]
#[case::ipv6_default("https", "[::1]:443", "[::1]")]
#[case::ipv6_non_default("https", "[::1]:8443", "[::1]:8443")]
#[case::ipv6_without_port("https", "[::1]", "[::1]")]
#[case::non_numeric_port("https", "h:abc", "h:abc")]
#[case::empty_port("https", "h:", "h:")]
#[case::signed_port("https", "h:+443", "h")]
#[case::spaced_port("https", "h: 443", "h")]
#[case::padded_port("https", "h:0443", "h")]
#[case::underscored_port("https", "h:443_0", "h:443_0")]
#[case::single_underscore_default("https", "h:4_43", "h")]
#[case::leading_underscore("https", "h:_443", "h:_443")]
#[case::double_underscore("https", "h:44__3", "h:44__3")]
#[case::trailing_underscore("https", "h:443_", "h:443_")]
#[case::double_sign("https", "h:--443", "h:--443")]
#[case::lone_sign("https", "h:-", "h:-")]
#[case::other_scheme("ftp", "h:21", "h:21")]
#[case::unknown_scheme("https", "h:80", "h:80")]
#[case::empty_netloc("https", "", "")]
fn strip_default_port_matches_litellm(
    #[case] scheme: &str,
    #[case] netloc: &str,
    #[case] expected: &str,
) {
    assert_eq!(strip_default_port(scheme, netloc), expected);
}

#[test]
fn strip_default_port_keeps_an_uppercase_scheme_default() {
    assert_eq!(strip_default_port("HTTPS", "H:443"), "h:443");
}

#[rstest]
#[case::unset("")]
#[case::blank("   ")]
fn normalize_proxy_base_url_treats_blank_as_unset(#[case] raw: &str) {
    assert_eq!(normalize_proxy_base_url(raw), None);
}

#[rstest]
#[case::bare("https://proxy.example.com", "https://proxy.example.com")]
#[case::trailing_slash("https://proxy.example.com/", "https://proxy.example.com")]
#[case::path(
    "https://proxy.example.com/base/path/",
    "https://proxy.example.com/base/path"
)]
#[case::query_and_fragment(
    "https://proxy.example.com/base/path/?q=1#f",
    "https://proxy.example.com/base/path"
)]
#[case::params_keep("http://h:8080/a;b/", "http://h:8080/a;b")]
#[case::mixed_case("HTTP://Proxy.Example.com/Path/", "http://Proxy.Example.com/Path")]
#[case::padded(" https://h/x ", "https://h/x")]
#[case::bad_port_text("https://h:abc/x", "https://h:abc/x")]
fn normalize_proxy_base_url_keeps_litellm_shapes(#[case] raw: &str, #[case] expected: &str) {
    assert_eq!(normalize_proxy_base_url(raw).as_deref(), Some(expected));
}

#[rstest]
#[case::other_scheme("ftp://h/x")]
#[case::no_scheme("proxy.example.com")]
#[case::scheme_relative("//h/x")]
#[case::no_host("https:///x")]
fn normalize_proxy_base_url_rejects_non_http_urls(#[case] raw: &str) {
    assert_eq!(normalize_proxy_base_url(raw), None);
}

#[test]
fn normalize_proxy_base_url_ignores_an_unparsable_url() {
    assert_eq!(normalize_proxy_base_url("https://[abc]/x"), None);
}

#[test]
fn untrusted_caller_gets_the_literal_base_url() {
    let request = origin(
        "http://llm.example.com:8000/root/",
        "https",
        "api.example.com",
        "8080",
        false,
    );
    assert_eq!(
        get_request_base_url("", &request).unwrap(),
        "http://llm.example.com:8000/root"
    );
}

#[test]
fn untrusted_caller_still_parses_the_base_url() {
    let request = origin("http://[abc]/root", "", "", "", false);
    assert_eq!(
        get_request_base_url("", &request).unwrap_err(),
        UrlParseError::InvalidIpv6
    );
}

#[rstest]
#[case::proto_host_port(
    "https://llm.example.com/",
    "https",
    "api.example.com",
    "443",
    "https://api.example.com"
)]
#[case::host_with_port_wins(
    "http://llm.example.com/",
    "https",
    "api.example.com:8443",
    "9999",
    "https://api.example.com:8443"
)]
#[case::host_and_port(
    "http://llm.example.com/",
    "",
    "api.example.com",
    "8080",
    "http://api.example.com:8080"
)]
#[case::ipv6_forwarded_host(
    "https://h/root",
    "",
    "[2001:db8::1]",
    "8080",
    "https://[2001:db8::1]:8080/root"
)]
#[case::empty_forwarded_values("https://h/root/", "", "", "", "https://h/root")]
#[case::host_with_empty_port("https://h/root/", "", "h2:", "", "https://h2:/root")]
#[case::uppercase_proto_keeps_port("https://h/root/", "HTTPS", "H:443", "", "HTTPS://h:443/root")]
fn trusted_caller_uses_forwarded_values(
    #[case] base_url: &str,
    #[case] proto: &str,
    #[case] host: &str,
    #[case] port: &str,
    #[case] expected: &str,
) {
    let request = origin(base_url, proto, host, port, true);
    assert_eq!(get_request_base_url("", &request).unwrap(), expected);
}

#[rstest]
#[case::plain_host(
    "http://llm.example.com/root/",
    "8080",
    "http://llm.example.com:8080/root"
)]
#[case::port_already_present(
    "http://llm.example.com:8000/root/",
    "8080",
    "http://llm.example.com:8000/root"
)]
#[case::ipv6_base_url("https://[::1]/root", "8080", "https://[::1]/root")]
fn forwarded_port_without_host_header(
    #[case] base_url: &str,
    #[case] port: &str,
    #[case] expected: &str,
) {
    let request = origin(base_url, "", "", port, true);
    assert_eq!(get_request_base_url("", &request).unwrap(), expected);
}

#[test]
fn forwarded_port_text_is_not_validated() {
    let request = origin("https://h/root/", "", "", " 8080", true);
    assert_eq!(
        get_request_base_url("", &request).unwrap(),
        "https://h: 8080/root"
    );
}

#[test]
fn configured_proxy_base_url_wins_over_headers() {
    let request = origin("http://h/", "https", "api.example.com", "8443", true);
    assert_eq!(
        get_request_base_url("https://proxy.example.com/", &request).unwrap(),
        "https://proxy.example.com"
    );
}

#[test]
fn configured_proxy_base_url_wins_for_an_untrusted_caller() {
    let request = origin("http://h/", "", "", "", false);
    assert_eq!(
        get_request_base_url("https://proxy.example.com", &request).unwrap(),
        "https://proxy.example.com"
    );
}

#[test]
fn malformed_trusted_base_url_is_a_typed_error() {
    let request = origin("https://[::1/root", "", "", "", true);
    assert_eq!(
        get_request_base_url("", &request).unwrap_err(),
        UrlParseError::InvalidIpv6
    );
}
