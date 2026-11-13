#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use tracing::{Subscriber, subscriber::set_default};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt as _};

use super::{
    get_litellm_api_key, mcp_auth_header, oauth2_headers, parse_mcp_access_groups_header,
    parse_mcp_servers_header, server_auth_headers,
};
use crate::request_headers::raw_headers::RawHeaders;

fn exposed(map: BTreeMap<String, SecretString>) -> BTreeMap<String, String> {
    map.into_iter()
        .map(|(name, value)| (name, value.expose_secret().to_owned()))
        .collect()
}

struct Warnings(Arc<Mutex<usize>>);

impl<Current> Layer<Current> for Warnings
where
    Current: Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Current>) {
        if *event.metadata().level() == tracing::Level::WARN {
            let mut count = self.0.lock().unwrap();
            *count = count.saturating_add(1);
        }
    }
}

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
    let key = get_litellm_api_key(&headers);
    assert_eq!(
        key.value.as_ref().map(ExposeSecret::expose_secret),
        Some("k1")
    );
    assert!(key.explicit_header_present);
}

#[test]
fn litellm_api_key_falls_back_to_authorization() {
    let headers = raw_headers(&[("Authorization", "Bearer token")]);
    let key = get_litellm_api_key(&headers);
    assert_eq!(
        key.value.as_ref().map(ExposeSecret::expose_secret),
        Some("Bearer token")
    );
    assert!(!key.explicit_header_present);
}

#[test]
fn litellm_api_key_reports_an_empty_explicit_header() {
    let headers = raw_headers(&[("x-litellm-api-key", ""), ("Authorization", "Bearer z")]);
    let key = get_litellm_api_key(&headers);
    assert_eq!(
        key.value.as_ref().map(ExposeSecret::expose_secret),
        Some("Bearer z")
    );
    assert!(key.explicit_header_present);
}

#[test]
fn litellm_api_key_is_absent_without_either_header() {
    let headers = raw_headers(&[]);
    let key = get_litellm_api_key(&headers);
    assert_eq!(key.value.as_ref().map(ExposeSecret::expose_secret), None);
    assert!(!key.explicit_header_present);
}

#[rstest]
#[case::matching_case("x-mcp-auth", Some("v"))]
#[case::configured_case("X-MCP-AUTH", Some("v"))]
#[case::absent("x-other-auth", None)]
fn mcp_auth_header_uses_the_configured_name(
    #[case] configured_name: &str,
    #[case] expected: Option<&str>,
) {
    let headers = raw_headers(&[("X-Mcp-Auth", "v")]);
    let value = mcp_auth_header(&headers, configured_name);
    assert_eq!(value.as_ref().map(ExposeSecret::expose_secret), expected);
}

#[rstest]
#[case::present(&[("x-mcp-auth", "v")], 1)]
#[case::empty(&[("x-mcp-auth", "")], 0)]
#[case::absent(&[], 0)]
fn mcp_auth_header_warns_only_for_a_non_empty_value(
    #[case] pairs: &[(&str, &str)],
    #[case] warnings: usize,
) {
    let count = Arc::new(Mutex::new(0_usize));
    let _guard = set_default(tracing_subscriber::registry().with(Warnings(Arc::clone(&count))));
    let value = mcp_auth_header(&raw_headers(pairs), "x-mcp-auth");
    assert_eq!(value.is_some(), !pairs.is_empty());
    assert_eq!(*count.lock().unwrap(), warnings);
}

#[test]
fn mcp_auth_header_keeps_an_empty_value() {
    let headers = raw_headers(&[("x-mcp-auth", "")]);
    let value = mcp_auth_header(&headers, "x-mcp-auth");
    assert_eq!(value.as_ref().map(ExposeSecret::expose_secret), Some(""));
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
    let actual: BTreeMap<String, BTreeMap<String, String>> = server_auth_headers(&headers)
        .into_iter()
        .map(|(alias, names)| (alias, exposed(names)))
        .collect();
    assert_eq!(actual, expected);
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
            .map(ExposeSecret::expose_secret),
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
        exposed(oauth2_headers(&headers)),
        BTreeMap::from([("Authorization".to_owned(), "last".to_owned())])
    );
}

#[test]
fn oauth2_headers_are_empty_without_an_authorization_header() {
    assert!(oauth2_headers(&raw_headers(&[("x-mcp-auth", "v")])).is_empty());
}

#[rstest]
#[case::servers_trimmed("x-mcp-servers", " s1 , ,s2 ", true, vec!["s1", "s2"])]
#[case::servers_empty("x-mcp-servers", "", true, vec![])]
#[case::servers_blank("x-mcp-servers", "   ", true, vec![])]
#[case::servers_commas("x-mcp-servers", ",,,", true, vec![])]
#[case::servers_absent("x-mcp-servers", "", false, vec![])]
#[case::groups_trimmed("x-mcp-access-groups", " g1,g2 ", true, vec!["g1", "g2"])]
#[case::groups_empty("x-mcp-access-groups", "", true, vec![])]
#[case::groups_absent("x-mcp-access-groups", "", false, vec![])]
fn comma_list_headers_match_litellm(
    #[case] raw_name: &str,
    #[case] value: &str,
    #[case] present: bool,
    #[case] expected: Vec<&str>,
) {
    let headers = if present {
        raw_headers(&[(raw_name, value)])
    } else {
        raw_headers(&[])
    };
    let wanted = present.then(|| names(&expected));
    let parsed = if raw_name == "x-mcp-servers" {
        parse_mcp_servers_header(&headers)
    } else {
        parse_mcp_access_groups_header(&headers)
    };
    assert_eq!(parsed, wanted);
}
