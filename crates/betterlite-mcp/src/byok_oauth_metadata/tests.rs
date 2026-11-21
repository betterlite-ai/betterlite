#![cfg(test)]

use rstest::rstest;

use super::{authorization_server_document, prefixed_base_url, protected_resource_document};
use crate::oauth_resource::python_url::UrlParseError;

#[test]
fn authorization_server_document_serializes_litellm_keys_in_order() {
    let document = authorization_server_document("https://proxy.test", "https://proxy.test");
    let serialized = serde_json::to_string(&document).unwrap();
    assert_eq!(
        serialized,
        concat!(
            r#"{"issuer":"https://proxy.test","#,
            r#""authorization_endpoint":"https://proxy.test/v1/mcp/oauth/authorize","#,
            r#""token_endpoint":"https://proxy.test/v1/mcp/oauth/token","#,
            r#""response_types_supported":["code"],"#,
            r#""grant_types_supported":["authorization_code"],"#,
            r#""code_challenge_methods_supported":["S256"]}"#,
        )
    );
}

#[test]
fn authorization_server_document_keeps_the_given_issuer() {
    let document = authorization_server_document(
        "https://proxy.test/prem",
        "https://proxy.test/prem/v1/mcp/oauth",
    );
    assert_eq!(document.issuer, "https://proxy.test/prem/v1/mcp/oauth");
    assert_eq!(
        document.authorization_endpoint,
        "https://proxy.test/prem/v1/mcp/oauth/authorize"
    );
    assert_eq!(
        document.token_endpoint,
        "https://proxy.test/prem/v1/mcp/oauth/token"
    );
}

#[test]
fn protected_resource_document_drops_the_base_path() {
    let document = protected_resource_document("https://proxy.test/root").unwrap();
    let serialized = serde_json::to_string(&document).unwrap();
    assert_eq!(
        serialized,
        concat!(
            r#"{"resource":"https://proxy.test","#,
            r#""authorization_servers":["https://proxy.test/root/v1/mcp/oauth"]}"#,
        )
    );
}

#[test]
fn protected_resource_document_keeps_the_netloc_text() {
    let document = protected_resource_document("https://Proxy.TEST:8443/api").unwrap();
    assert_eq!(document.resource, "https://Proxy.TEST:8443");
    assert_eq!(
        document.authorization_servers,
        ["https://Proxy.TEST:8443/api/v1/mcp/oauth".to_owned()]
    );
}

#[test]
fn protected_resource_document_uses_an_array_for_authorization_servers() {
    let document = protected_resource_document("https://proxy.test").unwrap();
    assert!(
        serde_json::to_string(&document)
            .unwrap()
            .contains(r#""authorization_servers":["https://proxy.test/v1/mcp/oauth"]"#)
    );
}

#[rstest]
#[case::with_path("https://proxy.test/root", "/prem", "https://proxy.test/prem")]
#[case::bare_base("https://proxy.test", "/prem", "https://proxy.test/prem")]
#[case::empty_prefix("https://proxy.test/root", "", "https://proxy.test")]
#[case::nested_prefix("http://10.0.0.1:8080/a/b", "/x/y", "http://10.0.0.1:8080/x/y")]
#[case::empty_base("", "/prem", "://prem")]
fn prefixed_base_url_keeps_only_scheme_and_netloc(
    #[case] base_url: &str,
    #[case] prefix: &str,
    #[case] expected: &str,
) {
    assert_eq!(prefixed_base_url(base_url, prefix).unwrap(), expected);
}

#[test]
fn prefixed_base_url_rejects_an_unterminated_ipv6_host() {
    assert_eq!(
        prefixed_base_url("http://[::1/prem", "/prem"),
        Err(UrlParseError::InvalidIpv6)
    );
}

#[test]
fn protected_resource_document_keeps_the_port_text_unparsed() {
    let document = protected_resource_document("http://proxy.test:bad").unwrap();
    assert_eq!(document.resource, "http://proxy.test:bad");
}

#[test]
fn protected_resource_document_rejects_an_unterminated_ipv6_host() {
    assert_eq!(
        protected_resource_document("http://[::1"),
        Err(UrlParseError::InvalidIpv6)
    );
}
