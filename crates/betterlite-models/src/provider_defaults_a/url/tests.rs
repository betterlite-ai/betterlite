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
#[case::scheme_without_authority(
    "https:app.example/api",
    parts(["https", "", "app.example/api", "", "", ""])
)]
#[case::protocol_relative(
    "//host.example/api",
    parts(["", "host.example", "/api", "", "", ""])
)]
#[case::relative_path(
    "app.example/api/v2",
    parts(["", "", "app.example/api/v2", "", "", ""])
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
fn unparse_joins_the_components_the_way_urllib_parse_does(
    #[case] source: &str,
    #[case] expected: &str,
) {
    let parsed = parse(source).unwrap();
    assert_eq!(unparse(&parsed), expected);
}

#[rstest]
#[case::host_only("https://tenant.datarobot.com", Some("tenant.datarobot.com"))]
#[case::host_and_port("https://tenant.datarobot.com:8443/v1", Some("tenant.datarobot.com"))]
#[case::userinfo_and_port("https://user:pass@host.example:8080/v1", Some("host.example"))]
#[case::bracketed_ipv6("https://[::1]:8080/v1", Some("::1"))]
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
#[case::empty_ipvfuture("https://[v1.]/api")]
fn parse_rejects_a_malformed_ipv6_authority(#[case] source: &str) {
    assert_eq!(parse(source).unwrap_err(), UrlError::InvalidIpv6);
}

#[rstest]
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

#[test]
fn parse_accepts_an_internationalized_host_that_nfkc_leaves_alone() {
    let parsed = parse("https://m\u{fc}nchen.example/v1").unwrap();
    assert_eq!(parsed.hostname(), Some("m\u{fc}nchen.example".to_owned()));
}
