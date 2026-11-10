#![cfg(test)]

mod auth_cases;
mod claims_cases;
mod fixture;
mod invariants;
mod irsa_cases;
mod replay;

use rstest::rstest;

#[derive(Debug, Clone, Copy)]
enum Scenario {
    JwtAudienceHint,
    WebIdentityAssumeRole,
    SecureTransportSessionPolicy,
    SameAccountIrsa,
    CrossAccountIrsa,
    MalformedToken,
    StsCredentialsMissing,
    InvalidIdentityTokenError,
    PackedPolicyWarningAndTtl,
}

#[rstest]
#[case::jwt_audience_hint(Scenario::JwtAudienceHint)]
#[case::web_identity_assume_role(Scenario::WebIdentityAssumeRole)]
#[case::secure_transport_session_policy(Scenario::SecureTransportSessionPolicy)]
#[case::same_account_irsa(Scenario::SameAccountIrsa)]
#[case::cross_account_irsa(Scenario::CrossAccountIrsa)]
#[case::malformed_token(Scenario::MalformedToken)]
#[case::sts_credentials_missing(Scenario::StsCredentialsMissing)]
#[case::invalid_identity_token_error(Scenario::InvalidIdentityTokenError)]
#[case::packed_policy_warning_and_ttl(Scenario::PackedPolicyWarningAndTtl)]
fn web_identity_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::JwtAudienceHint => claims_cases::jwt_audience_hint(),
        Scenario::WebIdentityAssumeRole => auth_cases::web_identity_assume_role(),
        Scenario::SecureTransportSessionPolicy => auth_cases::secure_transport_session_policy(),
        Scenario::SameAccountIrsa => irsa_cases::same_account_irsa(),
        Scenario::CrossAccountIrsa => irsa_cases::cross_account_irsa(),
        Scenario::MalformedToken => claims_cases::malformed_token(),
        Scenario::StsCredentialsMissing => auth_cases::sts_credentials_missing(),
        Scenario::InvalidIdentityTokenError => auth_cases::invalid_identity_token_error(),
        Scenario::PackedPolicyWarningAndTtl => auth_cases::packed_policy_warning_and_ttl(),
    }
}
