#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{CredError, CredResult, Unauthorized};

#[rstest]
fn result_ok_error() {
    let ok: CredResult<String> = Ok("token".to_owned());
    assert!(matches!(&ok, Ok(value) if value == "token"));

    let error: CredResult<String> = Err(CredError::Misconfigured("missing client_id".to_owned()));
    assert!(matches!(
        &error,
        Err(CredError::Misconfigured(message)) if message == "missing client_id"
    ));
}

#[rstest]
fn unauthorized_challenge_fields() {
    let mut body = BTreeMap::new();
    body.insert(
        "provisioning_url".to_owned(),
        "https://example.test".to_owned(),
    );
    let challenge = Unauthorized {
        detail: "no stored credential for this server".to_owned(),
        www_authenticate: Some("Bearer realm=\"mcp\"".to_owned()),
        body: Some(body.clone()),
        claims: Some("eyJjbGFpbXMiOiAxfQ".to_owned()),
    };
    let error = CredError::Unauthorized(challenge.clone());
    assert_eq!(
        error,
        CredError::Unauthorized(Unauthorized {
            detail: challenge.detail.clone(),
            www_authenticate: challenge.www_authenticate.clone(),
            body: Some(body),
            claims: challenge.claims,
        })
    );
    assert_eq!(
        error.summary(),
        "unauthorized: no stored credential for this server"
    );
}

#[rstest]
#[case::unauthorized(
    CredError::Unauthorized(Unauthorized {
        detail: "no credential".to_owned(),
        www_authenticate: None,
        body: None,
        claims: None,
    }),
    "unauthorized: no credential"
)]
#[case::misconfigured(
    CredError::Misconfigured("missing token_url".to_owned()),
    "misconfigured: missing token_url"
)]
#[case::url_credentials_not_allowed(
    CredError::UrlCredentialsNotAllowed,
    "misconfigured: auth_type none cannot be used with credentials embedded in the upstream URL; \
     remove them from the URL and configure Basic Auth with auth_type: basic and \
     auth_value: username:password"
)]
#[case::upstream_unavailable(
    CredError::UpstreamUnavailable("token endpoint timed out".to_owned()),
    "upstream unavailable: token endpoint timed out"
)]
#[case::unsupported_mode(
    CredError::UnsupportedMode("unknown auth_spec_kind: 'bogus'".to_owned()),
    "unknown auth_spec_kind: 'bogus'"
)]
#[case::precondition_required(
    CredError::PreconditionRequired("MCP_BYOK_API_KEY is not set".to_owned()),
    "precondition required: MCP_BYOK_API_KEY is not set"
)]
#[case::not_implemented(
    CredError::NotImplemented("token_exchange".to_owned()),
    "not implemented: token_exchange"
)]
fn cred_error_summary_matches_arm(#[case] error: CredError, #[case] expected: &str) {
    assert_eq!(error.summary(), expected);
}
