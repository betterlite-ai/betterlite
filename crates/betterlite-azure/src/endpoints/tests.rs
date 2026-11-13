#![cfg(test)]

mod cases;
mod credential_cases;
mod support;

use rstest::rstest;

#[rstest]
#[case::default_api_version(0)]
#[case::cloudflare_base_model(1)]
#[case::separate_token_client_identity(2)]
#[case::azure_sdk_client_mode(3)]
#[case::azure_initializer_credentials(4)]
#[case::azure_v1_client_projection(5)]
#[case::dynamic_api_version(6)]
fn endpoints_cases(#[case] case: u8) {
    match case {
        0 => cases::default_api_version(),
        1 => cases::cloudflare_base_model(),
        2 => cases::separate_token_client_identity(),
        3 => cases::azure_sdk_client_mode(),
        4 => cases::azure_initializer_credentials(),
        5 => cases::azure_v1_client_projection(),
        _ => cases::dynamic_api_version(),
    }
}

#[test]
fn azure_endpoints_credential_identity_isolation() {
    credential_cases::credential_identity_isolation();
}

#[test]
fn azure_endpoints_secret_safe_failure() {
    credential_cases::secret_safe_failure();
}
