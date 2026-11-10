#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::endpoint_target::headers::set_headers;
use crate::endpoint_target::target::{AuthHeader, EndpointTarget};

use super::support::{expected_headers, exposed, secret};

#[rstest]
#[case::only_content_type(None, &[], &[])]
#[case::bearer(Some("tok"), &[], &[("Authorization", "Bearer tok")])]
#[case::empty_bearer(Some(""), &[], &[("Authorization", "Bearer ")])]
#[case::content_type_override(None, &[("Content-Type", "text/plain")], &[("Content-Type", "text/plain")])]
#[case::authorization_override(Some("tok"), &[("Authorization", "Custom x")], &[("Authorization", "Custom x")])]
#[case::extra_header(Some("tok"), &[("X-Extra", "1")], &[("Authorization", "Bearer tok"), ("X-Extra", "1")])]
#[case::lowercase_content_type(Some("tok"), &[("content-type", "text/plain")], &[("Authorization", "Bearer tok"), ("content-type", "text/plain")])]
fn set_headers_cases(
    #[case] auth: Option<&str>,
    #[case] extras: &[(&str, &str)],
    #[case] expected_extras: &[(&str, &str)],
) {
    let token = auth.map(secret);
    let extra_headers: BTreeMap<String, String> = extras
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let headers = set_headers(token.as_ref(), &extra_headers);
    let exposed_headers: BTreeMap<String, String> = headers
        .iter()
        .map(|(name, value)| (name.clone(), exposed(value).to_owned()))
        .collect();
    assert_eq!(exposed_headers, expected_headers(expected_extras));
}

#[test]
fn endpoint_target_debug_redacts_credentials() {
    let auth = AuthHeader::Token(secret("tok-secret"));
    let target = EndpointTarget {
        url: "https://example.com".to_owned(),
        auth: Some(auth),
    };
    let debug = format!("{target:?}");
    assert!(!debug.contains("tok-secret"));
    assert_eq!(super::support::token_of(&target), Some("tok-secret"));
}
