#![cfg(test)]

mod cases;
mod support;

use rstest::rstest;

#[rstest]
#[case::static_ad_token(0)]
#[case::client_secret_precedence(1)]
#[case::username_password(2)]
#[case::refresh_fallback(3)]
#[case::header_case_insensitive_preservation(4)]
#[case::redacted_auth_headers(5)]
#[case::azure_response_headers(6)]
fn credentials_cases(#[case] case: u8) {
    match case {
        0 => cases::static_ad_token(),
        1 => cases::client_secret_precedence(),
        2 => cases::username_password(),
        3 => {
            cases::refresh_fallback();
            cases::refresh_provider_cache();
        }
        4 => cases::header_case_insensitive_preservation(),
        5 => cases::redacted_auth_headers(),
        _ => cases::azure_response_headers(),
    }
}

#[test]
fn azure_credentials_credential_identity_isolation() {
    cases::identity_isolation();
}

#[test]
fn azure_credentials_secret_safe_failure() {
    cases::secret_safe_failure();
}
