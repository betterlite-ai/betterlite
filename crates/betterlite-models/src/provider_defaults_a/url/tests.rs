#![cfg(test)]

use rstest::rstest;

use crate::provider_defaults_a::url::{UrlError, UrlParts, parse, unparse};

fn parts([scheme, netloc, path, params, query, fragment]: [&str; 6]) -> UrlParts {
    UrlParts {
        scheme: scheme.to_owned(),
        netloc: netloc.to_owned(),
        path: path.to_owned(),
        params: params.to_owned(),
        query: query.to_owned(),
        fragment: fragment.to_owned(),
    }
}

#[rstest]
#[case::absolute_url(
    "https://tenant.datarobot.com/api/v2",
    parts(["https", "tenant.datarobot.com", "/api/v2", "", "", ""])
)]
#[case::uppercase_scheme(
    "HTTPS://Tenant.DataRobot.com/api/v2",
    parts(["https", "Tenant.DataRobot.com", "/api/v2", "", "", ""])
)]
#[case::query_and_fragment(
    "https://h.example/api/v2?x=1#frag",
    parts(["https", "h.example", "/api/v2", "", "x=1", "frag"])
)]
#[case::params_before_query(
    "https://h.example/api/v2;param?q=1",
    parts(["https", "h.example", "/api/v2", "param", "q=1", ""])
)]
#[case::semicolon_before_the_last_slash_is_path_text(
    "https://h.example/a;b/c",
    parts(["https", "h.example", "/a;b/c", "", "", ""])
)]
#[case::scheme_without_authority(
    "https:app.example/api",
    parts(["https", "", "app.example/api", "", "", ""])
)]
#[case::not_a_scheme_before_the_colon(
    "1http:app.example/api",
    parts(["", "", "1http:app.example/api", "", "", ""])
)]
#[case::scheme_with_plus(
    "svn+ssh://host.example/x",
    parts(["svn+ssh", "host.example", "/x", "", "", ""])
)]
#[case::protocol_relative(
    "//host.example/api",
    parts(["", "host.example", "/api", "", "", ""])
)]
#[case::relative_path(
    "app.example/api/v2",
    parts(["", "", "app.example/api/v2", "", "", ""])
)]
#[case::tab_and_newline_are_removed(
    "https://h.example/a\tb\nc",
    parts(["https", "h.example", "/abc", "", "", ""])
)]
#[case::empty("", parts(["", "", "", "", "", ""]))]
fn parse_splits_the_components_urllib_parse_reports(
    #[case] source: &str,
    #[case] expected: UrlParts,
) {
    assert_eq!(parse(source), Ok(expected));
}

#[rstest]
#[case::absolute_url(
    "https://tenant.datarobot.com/api/v2",
    "https://tenant.datarobot.com/api/v2"
)]
#[case::uppercase_scheme(
    "HTTPS://Tenant.DataRobot.com/api/v2",
    "https://Tenant.DataRobot.com/api/v2"
)]
#[case::query_and_fragment(
    "https://h.example/api/v2?x=1#frag",
    "https://h.example/api/v2?x=1#frag"
)]
#[case::params_before_query(
    "https://h.example/api/v2;param?q=1",
    "https://h.example/api/v2;param?q=1"
)]
#[case::scheme_without_authority("https:app.example/api", "https:app.example/api")]
#[case::protocol_relative("//host.example/api", "//host.example/api")]
#[case::relative_path("app.example/api/v2", "app.example/api/v2")]
#[case::relative_absolute_path("/api", "/api")]
#[case::empty_authority_with_scheme("https:", "https://")]
#[case::empty_authority_with_path("https:/api", "https:///api")]
#[case::scheme_outside_the_netloc_list("custom:/x", "custom:/x")]
#[case::host_without_a_path("https://host", "https://host")]
fn unparse_joins_the_components_the_way_urllib_parse_does(
    #[case] source: &str,
    #[case] expected: &str,
) {
    let parsed = parse(source).unwrap();
    assert_eq!(unparse(&parsed), expected);
}

#[test]
fn unparse_adds_the_slash_before_a_relative_path_under_an_authority() {
    let parts = parts(["", "host", "abc", "", "", ""]);
    assert_eq!(unparse(&parts), "//host/abc");
}

#[rstest]
#[case::host_only("https://tenant.datarobot.com", Some("tenant.datarobot.com"))]
#[case::host_and_port("https://tenant.datarobot.com:8443/v1", Some("tenant.datarobot.com"))]
#[case::userinfo_and_port("https://user:pass@host.example:8080/v1", Some("host.example"))]
#[case::bracketed_ipv6("https://[::1]:8080/v1", Some("::1"))]
#[case::bracketed_ipv6_without_port("https://[::1]/v1", Some("::1"))]
#[case::uppercase_host("https://Tenant.DataRobot.com", Some("tenant.datarobot.com"))]
#[case::path_only("/v1/models", None)]
#[case::empty("", None)]
fn hostname_matches_urllib_parse(#[case] source: &str, #[case] expected: Option<&str>) {
    let parsed = parse(source).unwrap();
    assert_eq!(parsed.hostname(), expected.map(str::to_owned));
}

#[rstest]
#[case::open_without_close("https://[::1/api")]
#[case::close_without_open("https://host]/api")]
#[case::data_before_bracket("https://user@host[::1]/api")]
#[case::data_after_bracket("https://[::1]x/api")]
#[case::not_an_address("https://[abc]/api")]
#[case::empty_scope_id("https://[fe80::1%]/api")]
#[case::second_percent_in_scope_id("https://[fe80::1%a%b]/api")]
#[case::empty_ipvfuture("https://[v1.]/api")]
#[case::non_hex_ipvfuture("https://[vg.fe]/api")]
fn parse_rejects_a_malformed_ipv6_authority(#[case] source: &str) {
    assert_eq!(parse(source).unwrap_err(), UrlError::InvalidIpv6);
}

#[rstest]
#[case::ipv6("https://[::1]/api", "::1")]
#[case::scope_id("https://[fe80::1%25eth0]:8080/v1", "fe80::1%25eth0")]
#[case::ipvfuture("https://[v1.fe]:8080/v1", "v1.fe")]
fn parse_accepts_the_bracketed_forms_ipaddress_accepts(
    #[case] source: &str,
    #[case] expected_host: &str,
) {
    let parsed = parse(source).unwrap();
    assert_eq!(parsed.hostname(), Some(expected_host.to_owned()));
}

#[rstest]
#[case::nfkc_delimiter("https://a\u{2100}b.example/v1")]
#[case::nfkc_delimiter_with_port("https://a\u{2100}b.example:8080/v1")]
fn parse_rejects_a_netloc_whose_nfkc_form_readds_a_delimiter(#[case] source: &str) {
    assert_eq!(parse(source).unwrap_err(), UrlError::InvalidNetloc);
}

#[rstest]
#[case::internationalized_host("https://m\u{fc}nchen.example/v1", "m\u{fc}nchen.example")]
#[case::nfkc_change_without_a_delimiter("https://\u{fb01}.example/v1", "\u{fb01}.example")]
fn parse_accepts_a_netloc_whose_nfkc_form_stays_a_host(
    #[case] source: &str,
    #[case] expected_host: &str,
) {
    let parsed = parse(source).unwrap();
    assert_eq!(parsed.hostname(), Some(expected_host.to_owned()));
}
