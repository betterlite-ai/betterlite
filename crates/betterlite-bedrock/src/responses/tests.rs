#![cfg(test)]

use rstest::rstest;

mod auth_cases;
mod codex_cases;
mod image_cases;
mod option_cases;
mod route_cases;
mod support;
mod url_cases;

#[derive(Debug, Clone, Copy)]
enum Scenario {
    ModelRoute,
    BearerAuth,
    Sigv4,
    OptionGates,
    NativeFileSearch,
    Traversal,
    RemoteUrlInlined,
    RuntimeUrl,
    Replacement,
    CodexRewrite,
}

#[rstest]
#[case::bedrock_responses_model_route(Scenario::ModelRoute)]
#[case::responses_bearer_auth(Scenario::BearerAuth)]
#[case::responses_sigv4(Scenario::Sigv4)]
#[case::response_option_gates(Scenario::OptionGates)]
#[case::native_file_search_flag(Scenario::NativeFileSearch)]
#[case::inline_image_traversal(Scenario::Traversal)]
#[case::remote_url_inlined(Scenario::RemoteUrlInlined)]
#[case::responses_runtime_url(Scenario::RuntimeUrl)]
#[case::inline_image_replacement(Scenario::Replacement)]
#[case::codex_input_rewrite(Scenario::CodexRewrite)]
fn responses_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::ModelRoute => route_cases::bedrock_responses_model_route(),
        Scenario::BearerAuth => auth_cases::responses_bearer_auth(),
        Scenario::Sigv4 => auth_cases::responses_sigv4(),
        Scenario::OptionGates => option_cases::response_option_gates(),
        Scenario::NativeFileSearch => option_cases::native_file_search_flag(),
        Scenario::Traversal => image_cases::inline_image_traversal(),
        Scenario::RemoteUrlInlined => image_cases::remote_url_inlined(),
        Scenario::RuntimeUrl => url_cases::responses_runtime_url(),
        Scenario::Replacement => image_cases::inline_image_replacement(),
        Scenario::CodexRewrite => codex_cases::codex_input_rewrite(),
    }
}

#[test]
fn responses_media_fetch() {
    image_cases::responses_media_fetch();
}

#[test]
fn remote_url_limit() {
    image_cases::remote_url_limit();
}

#[test]
fn runtime_responses_credential_identity_isolation() {
    auth_cases::credential_identity_isolation();
}

#[test]
fn runtime_responses_secret_safe_failure() {
    auth_cases::secret_safe_failure();
}
