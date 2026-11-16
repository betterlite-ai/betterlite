#![cfg(test)]

mod cases;
mod identity;
mod secret_safety;
mod support;

use rstest::rstest;

#[rstest]
#[case::foundry_responses_url(0)]
#[case::responses_auth(1)]
#[case::missing_base(2)]
#[case::no_native_websocket(3)]
#[case::decisions_route(4)]
fn responses_cases(#[case] case: u8) {
    match case {
        0 => cases::url_case::foundry_responses_url(),
        1 => cases::auth_case::responses_auth(),
        2 => cases::missing_base_case::missing_base(),
        3 => cases::capability_case::no_native_websocket(),
        _ => cases::decisions_case::decisions_route(),
    }
}

#[test]
fn foundry_responses_credential_identity_isolation() {
    identity::credential_identity_isolation();
}

#[test]
fn foundry_responses_secret_safe_failure() {
    secret_safety::secret_safe_failure();
}
