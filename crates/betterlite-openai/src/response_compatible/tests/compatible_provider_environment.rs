use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;
use secrecy::SecretString;

use super::{config, header_map};

// provider_capability_selection's compatible fallback: openai_like identity, native file search, no encrypted messages
#[test]
fn identity_is_openai_like() {
    let capabilities = config(&[]).identity();
    assert_eq!(capabilities.custom_llm_provider, ProviderId::OpenaiLike);
    assert!(capabilities.supports_native_file_search);
    assert!(!capabilities.encrypted_agent_messages);
}

#[rstest]
#[case::env_absent_no_caller(&[], None, None, None)]
#[case::env_absent_keeps_caller(&[], None, Some("Bearer caller"), Some("Bearer caller"))]
#[case::explicit_key_beats_env(&[("OPENAI_LIKE_API_KEY", "sk-env")], Some("sk-explicit"), Some("Bearer caller"), Some("Bearer sk-explicit"))]
#[case::empty_key_uses_env(&[("OPENAI_LIKE_API_KEY", "sk-env")], Some(""), None, Some("Bearer sk-env"))]
#[case::environment_key(&[("OPENAI_LIKE_API_KEY", "sk-env")], None, None, Some("Bearer sk-env"))]
#[case::whitespace_key_is_kept(&[], Some(" "), None, Some("Bearer  "))]
fn headers_follow_key_precedence(
    #[case] environment: &[(&str, &str)],
    #[case] api_key: Option<&str>,
    #[case] caller_header: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let key = api_key.map(SecretString::from);
    let start = caller_header
        .map(|header| header_map(&[("Authorization", header)]))
        .unwrap_or_default();
    let headers = config(environment).validate_environment(start, key.as_ref());
    assert_eq!(headers.get("Authorization").map(String::as_str), expected);
}

#[rstest]
#[case::explicit_base_beats_env(&[("OPENAI_LIKE_API_BASE", "https://env.test/v1")], Some("https://provider.test/v1///"), Ok("https://provider.test/v1/responses"))]
#[case::empty_base_uses_env(&[("OPENAI_LIKE_API_BASE", "https://env.test/v1")], Some(""), Ok("https://env.test/v1/responses"))]
#[case::absent_base_uses_env(&[("OPENAI_LIKE_API_BASE", "https://env.test/v1")], None, Ok("https://env.test/v1/responses"))]
#[case::missing_base(&[], None, Err(super::super::CompatibleConfigError::MissingApiBase))]
#[case::whitespace_base_is_kept(&[], Some(" "), Ok(" /responses"))]
#[case::responses_path_is_doubled(&[], Some("https://provider.test/responses"), Ok("https://provider.test/responses/responses"))]
#[case::query_is_kept(&[], Some("https://provider.test/v1?x=1"), Ok("https://provider.test/v1?x=1/responses"))]
fn urls_always_end_in_responses(
    #[case] environment: &[(&str, &str)],
    #[case] api_base: Option<&str>,
    #[case] expected: Result<&str, super::super::CompatibleConfigError>,
) {
    let outcome = config(environment).get_complete_url(api_base);
    assert_eq!(outcome, expected.map(str::to_owned));
}
