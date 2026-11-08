#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{
    LitellmApiKey, get_litellm_api_key, mcp_auth_header, oauth2_headers,
    parse_mcp_access_groups_header, parse_mcp_servers_header, server_auth_headers,
};
use crate::request_headers::raw_headers::RawHeaders;

fn raw_headers(pairs: &[(&str, &str)]) -> RawHeaders {
    let bytes: Vec<(&[u8], &[u8])> = pairs
        .iter()
        .map(|(name, value)| (name.as_bytes(), value.as_bytes()))
        .collect();
    RawHeaders::from_pairs(&bytes).expect("valid headers")
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn litellm_api_key_prefers_the_explicit_header() {
    let headers = raw_headers(&[
        ("x-litellm-api-key", "k1"),
        ("Authorization", "Bearer token"),
    ]);
    assert_eq!(
        get_litellm_api_key(&headers),
        LitellmApiKey {
            value: Some("k1"),
            explicit_header_present: true,
        }
    );
}

#[test]
fn litellm_api_key_falls_back_to_authorization() {
    let headers = raw_headers(&[("Authorization", "Bearer token")]);
    assert_eq!(
        get_litellm_api_key(&headers),
        LitellmApiKey {
            value: Some("Bearer token"),
            explicit_header_present: false,
        }
    );
}

#[test]
fn litellm_api_key_reports_an_empty_explicit_header() {
    let headers = raw_headers(&[("x-litellm-api-key", ""), ("Authorization", "Bearer z")]);
    assert_eq!(
        get_litellm_api_key(&headers),
        LitellmApiKey {
            value: Some("Bearer z"),
            explicit_header_present: true,
        }
    );
}

#[test]
fn litellm_api_key_is_absent_without_either_header() {
    assert_eq!(
        get_litellm_api_key(&raw_headers(&[])),
        LitellmApiKey {
            value: None,
            explicit_header_present: false,
        }
    );
}

#[rstest]
#[case::matching_case("x-mcp-auth", Some("v"))]
#[case::configured_case("X-MCP-AUTH", Some("v"))]
#[case::empty_value("x-mcp-auth", Some(""))]
#[case::absent("x-other-auth", None)]
fn mcp_auth_header_uses_the_configured_name(
    #[case] configured_name: &str,
    #[case] expected: Option<&str>,
) {
    let headers = raw_headers(&[("X-Mcp-Auth", "v")]);
    assert_eq!(mcp_auth_header(&headers, configured_name), expected);
}

#[test]
fn server_auth_headers_parse_the_alias_and_header_name() {
    let headers = raw_headers(&[
        ("X-Mcp-GitHub-Authorization", "Bearer t"),
        ("x-mcp-zapier-x-api-key", "api"),
        ("x-mcp-my-server-authorization", "B"),
        ("x-mcp-servers", "ignored"),
        ("x-mcp-access-groups", "ignored"),
        ("x-mcp-x", "ignored"),
    ]);
    let mut expected: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    expected.insert(
        "github".to_owned(),
        BTreeMap::from([("Authorization".to_owned(), "Bearer t".to_owned())]),
    );
    expected.insert(
        "zapier".to_owned(),
        BTreeMap::from([("x-api-key".to_owned(), "api".to_owned())]),
    );
    expected.insert(
        "my".to_owned(),
        BTreeMap::from([("server-authorization".to_owned(), "B".to_owned())]),
    );
    assert_eq!(server_auth_headers(&headers), expected);
}

#[test]
fn server_auth_headers_overwrite_the_same_alias_and_name() {
    let headers = raw_headers(&[
        ("x-mcp-github-authorization", "first"),
        ("x-mcp-github-authorization", "second"),
    ]);
    assert_eq!(
        server_auth_headers(&headers)
            .get("github")
            .and_then(|names| names.get("Authorization"))
            .map(String::as_str),
        Some("second")
    );
}

#[test]
fn oauth2_headers_keep_the_last_authorization_prefixed_header() {
    let headers = raw_headers(&[
        ("Authorization", "Bearer first"),
        ("authorization-foo", "last"),
    ]);
    assert_eq!(
        oauth2_headers(&headers),
        BTreeMap::from([("Authorization".to_owned(), "last".to_owned())])
    );
}

#[test]
fn oauth2_headers_are_empty_without_an_authorization_header() {
    assert!(oauth2_headers(&raw_headers(&[("x-mcp-auth", "v")])).is_empty());
}

#[rstest]
#[case::trimmed(" s1 , ,s2 ", true, vec!["s1", "s2"])]
#[case::empty("", true, vec![])]
#[case::blank("   ", true, vec![])]
#[case::commas(",,,", true, vec![])]
#[case::absent("", false, vec![])]
fn mcp_servers_header_matches_litellm(
    #[case] value: &str,
    #[case] present: bool,
    #[case] expected: Vec<&str>,
) {
    let headers = if present {
        raw_headers(&[("x-mcp-servers", value)])
    } else {
        raw_headers(&[])
    };
    let wanted = present.then(|| names(&expected));
    assert_eq!(parse_mcp_servers_header(&headers), wanted);
}

#[rstest]
#[case::trimmed(" g1,g2 ", true, vec!["g1", "g2"])]
#[case::empty("", true, vec![])]
#[case::absent("", false, vec![])]
fn mcp_access_groups_header_matches_litellm(
    #[case] value: &str,
    #[case] present: bool,
    #[case] expected: Vec<&str>,
) {
    let headers = if present {
        raw_headers(&[("x-mcp-access-groups", value)])
    } else {
        raw_headers(&[])
    };
    let wanted = present.then(|| names(&expected));
    assert_eq!(parse_mcp_access_groups_header(&headers), wanted);
}
