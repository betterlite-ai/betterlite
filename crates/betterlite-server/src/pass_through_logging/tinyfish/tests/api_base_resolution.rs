#![cfg(test)]

use betterlite_config::loader::startup::process_environment;
use rstest::rstest;

use super::resolve_tinyfish_api_base;
use super::settings::api_base_from_environment;
use super::support::environment;

#[rstest]
#[case::unset(&[], "https://agent.tinyfish.ai")]
#[case::an_empty_value(&[("TINYFISH_AGENT_API_BASE", "")], "https://agent.tinyfish.ai")]
#[case::another_variable(&[("TINYFISH_COST_PER_STEP", "0.5")], "https://agent.tinyfish.ai")]
#[case::a_base_with_a_scheme(
    &[("TINYFISH_AGENT_API_BASE", "https://agent.example.test")],
    "https://agent.example.test"
)]
#[case::a_base_with_another_scheme(
    &[("TINYFISH_AGENT_API_BASE", "ftp://agent.example.test")],
    "ftp://agent.example.test"
)]
#[case::a_host_only_base(
    &[("TINYFISH_AGENT_API_BASE", "agent.example.test")],
    "https://agent.example.test"
)]
#[case::a_host_only_base_with_a_port(
    &[("TINYFISH_AGENT_API_BASE", "agent.example.test:8443")],
    "https://agent.example.test:8443"
)]
#[case::one_trailing_slash(
    &[("TINYFISH_AGENT_API_BASE", "https://agent.example.test/")],
    "https://agent.example.test"
)]
#[case::every_trailing_slash(
    &[("TINYFISH_AGENT_API_BASE", "https://agent.example.test//")],
    "https://agent.example.test"
)]
#[case::a_lone_scheme(
    &[("TINYFISH_AGENT_API_BASE", "https://")],
    "https://https:"
)]
#[case::surrounding_spaces_kept(
    &[("TINYFISH_AGENT_API_BASE", " agent.example.test ")],
    "https:// agent.example.test "
)]
fn resolves_the_agent_api_base(#[case] entries: &[(&str, &str)], #[case] expected: &str) {
    assert_eq!(api_base_from_environment(&environment(entries)), expected);
}

#[test]
fn the_public_base_follows_the_process_environment() {
    assert_eq!(
        resolve_tinyfish_api_base(),
        api_base_from_environment(&process_environment())
    );
}
