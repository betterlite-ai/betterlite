#![cfg(test)]

mod cases;
mod support;

use rstest::rstest;

#[rstest]
#[case::case_1_workspace_id_required(cases::workspace_id_required)]
#[case::case_2_chat_platform_headers(cases::chat_platform_headers)]
#[case::case_3_native_platform_headers(cases::native_platform_headers)]
#[case::case_4_chat_platform_body(|| cases::platform_body(cases::BodyRoute::Chat))]
#[case::case_5_native_platform_body(|| cases::platform_body(cases::BodyRoute::Native))]
#[case::case_6_native_anthropic_stream(cases::native_anthropic_stream)]
fn claude_platform_cases(#[case] case: fn()) {
    case();
}

#[test]
fn bedrock_services_claude_platform_credential_identity_isolation() {
    cases::credential_identity_isolation();
}

#[test]
fn bedrock_services_claude_platform_secret_safe_failure() {
    cases::secret_safe_failure();
}

#[test]
fn bedrock_services_claude_platform_native_transform_status() {
    cases::native_transform_status();
}
