#![cfg(test)]

mod agent_cases;
mod auth_cases;
mod builders;
mod host_cases;
mod invariants;
mod native_cases;
mod settings_cases;
mod support;

use rstest::rstest;

#[rstest]
#[case::foundry_host_boundary(0)]
#[case::openai_v1_api_key_header(1)]
#[case::explicit_key_before_entra(2)]
#[case::missing_credentials(3)]
#[case::agent_entra_scope(4)]
#[case::agent_config_only_credentials(5)]
#[case::native_responses_eligibility(6)]
#[case::foundry_settings(7)]
#[case::foundry_reasoning_tool_gate(8)]
fn credentials_cases(#[case] case: u8) {
    match case {
        0 => host_cases::foundry_host_boundary(),
        1 => host_cases::openai_v1_api_key_header(),
        2 => auth_cases::explicit_key_before_entra(),
        3 => auth_cases::missing_credentials(),
        4 => agent_cases::agent_entra_scope(),
        5 => agent_cases::agent_config_only_credentials(),
        6 => native_cases::native_responses_eligibility(),
        7 => settings_cases::foundry_settings(),
        _ => native_cases::foundry_reasoning_tool_gate(),
    }
}

#[test]
fn foundry_credentials_credential_identity_isolation() {
    invariants::credential_identity_isolation();
}

#[test]
fn foundry_credentials_secret_safe_failure() {
    invariants::secret_safe_failure();
}
