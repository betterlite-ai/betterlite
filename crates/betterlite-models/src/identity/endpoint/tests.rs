#![cfg(test)]

use super::endpoint_matches_api_base;
use rstest::rstest;

#[rstest]
#[case::bare_host_matches_bare_host("api.groq.com", "api.groq.com", true)]
#[case::bare_host_matches_full_url("api.groq.com", "https://api.groq.com/openai/v1", true)]
#[case::host_is_case_insensitive("api.groq.com", "https://API.GROQ.COM", true)]
#[case::host_with_path_matches_exact_path(
    "api.deepinfra.com/v1/openai",
    "https://api.deepinfra.com/v1/openai",
    true
)]
#[case::host_with_path_matches_a_deeper_path(
    "api.deepinfra.com/v1/openai",
    "https://api.deepinfra.com/v1/openai/chat/completions",
    true
)]
#[case::trailing_slash_on_either_side_is_ignored(
    "api.deepinfra.com/v1/openai/",
    "https://api.deepinfra.com/v1/openai",
    true
)]
#[case::different_host_does_not_match("api.groq.com", "https://api.openai.com", false)]
#[case::host_used_as_an_attacker_path_segment_does_not_match(
    "api.groq.com",
    "https://attacker.com/api.groq.com/openai/v1",
    false
)]
#[case::a_path_that_is_a_string_suffix_but_not_a_segment_does_not_match(
    "api.deepinfra.com/v1/openai",
    "https://api.deepinfra.com/v1/openaiiiiii",
    false
)]
#[case::a_shorter_path_does_not_satisfy_a_longer_registered_path(
    "api.deepinfra.com/v1/openai",
    "https://api.deepinfra.com/v1",
    false
)]
#[case::an_endpoint_with_no_host_never_matches("", "https://api.groq.com", false)]
fn provider_identity_behavior_tests_behavior(
    #[case] endpoint: &str,
    #[case] api_base: &str,
    #[case] expected: bool,
) {
    assert_eq!(endpoint_matches_api_base(endpoint, api_base), expected);
}
