use std::collections::BTreeMap;

use rstest::rstest;

use crate::credential_contracts::errors::{CredError, Unauthorized};
use crate::server_record::MCPServer;
use crate::wire_compat::errors::HttpException;

use super::super::adapter::challenge::{
    oauth_protected_resource_path, raise_token_exchange_challenge, raise_user_oauth_challenge,
};
use super::super::adapter::errors::raise_public;
use super::support::server_from;

fn server_with(extra: &str) -> MCPServer {
    let encoded = format!(
        r#"{{"server_id":"srv-1","name":"Server One","transport":"http","url":"https://mcp.example.com/mcp"{extra}}}"#
    );
    server_from(&encoded)
}

fn path(root_path: &str, scalar_root_path: &str, server: &MCPServer) -> String {
    oauth_protected_resource_path(root_path, scalar_root_path, server)
}

fn challenge_header(exception: &HttpException) -> String {
    let challenge = exception
        .headers
        .get("www-authenticate")
        .expect("a challenge header");
    challenge.clone()
}

#[test]
fn resource_path_scalar_root_path() {
    let record = server_with(r#","alias":"Alias""#);
    assert_eq!(
        path("/litellm", "/litellm", &record),
        "/.well-known/oauth-protected-resource/litellm/mcp/Alias"
    );
    assert_eq!(
        path("/litellm", "/litellm/", &record),
        "/.well-known/oauth-protected-resource/litellm/mcp/Alias"
    );
    assert_eq!(
        path("/litellm", "/other", &record),
        "/litellm/.well-known/oauth-protected-resource/mcp/Alias"
    );
}

#[test]
fn resource_path_per_request_prefix() {
    let record = server_with(r#","server_name":"named""#);
    assert_eq!(
        path("/team/a", "", &record),
        "/team/a/.well-known/oauth-protected-resource/mcp/named"
    );
    assert_eq!(
        path("/team/a", "/litellm", &record),
        "/team/a/.well-known/oauth-protected-resource/mcp/named"
    );
    assert_eq!(
        path("litellm", "", &record),
        "litellm/.well-known/oauth-protected-resource/mcp/named"
    );
}

#[test]
fn resource_path_well_known_insertion_scalar() {
    let record = server_with(r#","alias":"","server_name":"""#);
    assert_eq!(
        path("/", "/litellm", &record),
        "/.well-known/oauth-protected-resource/mcp/Server One"
    );
    assert_eq!(
        path("", "", &record),
        "/.well-known/oauth-protected-resource/mcp/Server One"
    );
}

#[test]
fn resource_path_prefix_before_well_known_per_request() {
    let record = server_with(r#","alias":"Alias""#);
    assert_eq!(
        path("/team/a", "/litellm", &record),
        "/team/a/.well-known/oauth-protected-resource/mcp/Alias"
    );
    assert_eq!(
        path("/team/", "/team", &record),
        "/team//.well-known/oauth-protected-resource/mcp/Alias"
    );
    assert_eq!(
        path("/team/a", "/team/a/", &record),
        "/.well-known/oauth-protected-resource/team/a/mcp/Alias"
    );
}

#[test]
fn user_oauth_challenge_header() {
    let record = server_with(r#","alias":"Alias""#);
    let exception = raise_user_oauth_challenge(&record, "/", "");
    assert_eq!(exception.status_code, 401);
    assert_eq!(exception.detail, "Unauthorized");
    assert_eq!(exception.headers.len(), 1);
    assert_eq!(
        challenge_header(&exception),
        "Bearer resource_metadata=\"/.well-known/oauth-protected-resource/mcp/Alias\""
    );
}

#[test]
fn user_oauth_challenge_path_scalar() {
    let record = server_with(r#","alias":"Alias""#);
    let exception = raise_user_oauth_challenge(&record, "/litellm", "/litellm/");
    assert_eq!(
        challenge_header(&exception),
        "Bearer resource_metadata=\"/.well-known/oauth-protected-resource/litellm/mcp/Alias\""
    );
}

#[test]
fn user_oauth_challenge_path_per_request() {
    let record = server_with(r#","alias":"Alias""#);
    let exception = raise_user_oauth_challenge(&record, "/team/a", "/litellm");
    assert_eq!(
        challenge_header(&exception),
        "Bearer resource_metadata=\"/team/a/.well-known/oauth-protected-resource/mcp/Alias\""
    );
}

fn token_exchange_challenge(record: &MCPServer, claims: Option<&str>) -> String {
    let exception = raise_token_exchange_challenge(record, "/", "", claims);
    assert_eq!(exception.status_code, 401);
    assert_eq!(exception.detail, "Unauthorized");
    challenge_header(&exception)
}

#[test]
fn token_exchange_challenge_invalid_token() {
    let record = server_with(r#","alias":"Alias""#);
    let challenge = token_exchange_challenge(&record, None);
    assert_eq!(
        challenge,
        "Bearer resource_metadata=\"/.well-known/oauth-protected-resource/mcp/Alias\", error=\"invalid_token\", error_description=\"Missing or invalid subject token; authenticate with the IdP and retry\""
    );
    assert_eq!(token_exchange_challenge(&record, Some("")), challenge);
}

#[test]
fn token_exchange_challenge_insufficient_claims() {
    let record = server_with(r#","alias":"Alias""#);
    let challenge = token_exchange_challenge(&record, Some("a"));
    assert_eq!(
        challenge,
        "Bearer resource_metadata=\"/.well-known/oauth-protected-resource/mcp/Alias\", error=\"insufficient_claims\", error_description=\"Step-up authentication required; satisfy the returned claims challenge with the IdP and retry\", claims=\"YQ==\""
    );
}

#[rstest]
#[case::one_byte("a", "YQ==")]
#[case::two_bytes("ab", "YWI=")]
#[case::json_claims(r#"{"x":1}"#, "eyJ4IjoxfQ==")]
fn token_exchange_challenge_claims_base64(#[case] claims: &str, #[case] encoded: &str) {
    let record = server_with(r#","alias":"Alias""#);
    let challenge = token_exchange_challenge(&record, Some(claims));
    let expected = format!(
        "Bearer resource_metadata=\"/.well-known/oauth-protected-resource/mcp/Alias\", error=\"insufficient_claims\", error_description=\"Step-up authentication required; satisfy the returned claims challenge with the IdP and retry\", claims=\"{encoded}\""
    );
    assert_eq!(challenge, expected);
}

#[test]
fn token_exchange_challenge_error_description_varies() {
    let record = server_with(r#","alias":"Alias""#);
    let invalid = token_exchange_challenge(&record, None);
    let step_up = token_exchange_challenge(&record, Some("claims"));
    assert_ne!(invalid, step_up);
    assert!(
        invalid.contains("Missing or invalid subject token; authenticate with the IdP and retry")
    );
    assert!(step_up.contains(
        "Step-up authentication required; satisfy the returned claims challenge with the IdP and retry"
    ));
}

#[test]
fn raise_public_keeps_the_challenge_header() {
    let error = CredError::Unauthorized(Unauthorized {
        detail: "complete the flow".to_owned(),
        www_authenticate: Some("Bearer resource_metadata=\"/meta\"".to_owned()),
        body: None,
        claims: None,
    });
    let exception = raise_public(&error);
    assert_eq!(exception.status_code, 401);
    assert_eq!(exception.detail, error.to_string());
    let challenge = challenge_header(&exception);
    assert_eq!(challenge, "Bearer resource_metadata=\"/meta\"");
}

#[test]
fn raise_public_keeps_the_unauthorized_body_out_of_the_detail() {
    let error = CredError::Unauthorized(Unauthorized {
        detail: "complete the flow".to_owned(),
        www_authenticate: None,
        body: Some(BTreeMap::from([(
            "error".to_owned(),
            "invalid_request".to_owned(),
        )])),
        claims: None,
    });
    let exception = raise_public(&error);
    assert_eq!(exception.status_code, 401);
    assert_eq!(exception.detail, error.to_string());
    assert!(exception.headers.is_empty());
}

#[rstest]
#[case::misconfigured(CredError::Misconfigured("bad".to_owned()), 500_u16)]
#[case::upstream_unavailable(CredError::UpstreamUnavailable("down".to_owned()), 503_u16)]
#[case::unsupported_mode(CredError::UnsupportedMode("mode".to_owned()), 500_u16)]
#[case::precondition_required(CredError::PreconditionRequired("need".to_owned()), 412_u16)]
#[case::not_implemented(CredError::NotImplemented("todo".to_owned()), 501_u16)]
fn raise_public_maps_each_terminal_error(#[case] error: CredError, #[case] expected_status: u16) {
    let exception = raise_public(&error);
    assert_eq!(exception.status_code, expected_status);
    assert_eq!(exception.detail, error.to_string());
    assert!(exception.headers.is_empty());
}

#[test]
fn raise_public_maps_url_credentials() {
    let exception = raise_public(&CredError::UrlCredentialsNotAllowed);
    assert_eq!(exception.status_code, 500);
    assert_eq!(
        exception.detail,
        CredError::UrlCredentialsNotAllowed.to_string()
    );
    assert!(exception.headers.is_empty());
}
