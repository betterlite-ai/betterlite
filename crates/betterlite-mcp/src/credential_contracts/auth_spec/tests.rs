#![cfg(test)]

use rstest::rstest;

use super::{AuthResolution, AuthSpecKind, parse_auth_spec_kind};

#[rstest]
#[case::authorization_code("authorization_code", AuthSpecKind::AuthorizationCode)]
#[case::client_credentials("client_credentials", AuthSpecKind::ClientCredentials)]
#[case::token_exchange("token_exchange", AuthSpecKind::TokenExchange)]
#[case::id_jag("id_jag", AuthSpecKind::IdJag)]
#[case::api_key("api_key", AuthSpecKind::ApiKey)]
#[case::passthrough("passthrough", AuthSpecKind::Passthrough)]
#[case::none("none", AuthSpecKind::None)]
#[case::aws_sigv4("aws_sigv4", AuthSpecKind::AwsSigv4)]
fn parse_auth_spec_kind_success(#[case] raw: &str, #[case] expected: AuthSpecKind) {
    let parsed = parse_auth_spec_kind(raw).expect("known mode must parse");
    assert_eq!(parsed, expected);
    assert_eq!(parsed.to_string(), raw);
}

#[rstest]
fn parse_auth_spec_kind_unknown_mode_fail_closed() {
    let error = parse_auth_spec_kind("bogus").unwrap_err();
    assert_eq!(error.summary(), "unknown auth_spec_kind: 'bogus'");
}

#[rstest]
fn auth_resolution_enum() {
    let pairs = [
        (AuthResolution::NoAuth, "no-auth"),
        (AuthResolution::StoredUserToken, "stored-user-token"),
        (AuthResolution::StaticToken, "static-token"),
        (AuthResolution::PerRequestHeader, "per-request-header"),
        (AuthResolution::Oauth2Passthrough, "oauth2-passthrough"),
        (AuthResolution::ClientCredentials, "m2m-client-credentials"),
        (AuthResolution::TokenExchange, "token-exchange"),
        (AuthResolution::IdJag, "id-jag"),
        (AuthResolution::AwsSigv4, "aws-sigv4"),
        (AuthResolution::ExtraHeaders, "extra-headers"),
        (AuthResolution::NotApplicable, "not-applicable"),
        (AuthResolution::Unresolved, "unresolved"),
        (AuthResolution::Failed, "resolution-failed"),
        (AuthResolution::Multiple, "multiple"),
    ];
    for (resolution, expected) in pairs {
        assert_eq!(resolution.to_string(), expected);
    }
}
