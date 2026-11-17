#![cfg(test)]

use betterlite_config::loader::startup::process_environment;
use rstest::rstest;

use super::resolve_tinyfish_api_base;
use super::settings::{API_BASE_VARIABLE, api_base_from_text};

#[rstest]
#[case::unset(None, "https://agent.tinyfish.ai")]
#[case::an_empty_value(Some(""), "https://agent.tinyfish.ai")]
#[case::a_base_with_a_scheme(Some("https://agent.example.test"), "https://agent.example.test")]
#[case::a_base_with_another_scheme(Some("ftp://agent.example.test"), "ftp://agent.example.test")]
#[case::a_host_only_base(Some("agent.example.test"), "https://agent.example.test")]
#[case::a_host_only_base_with_a_port(
    Some("agent.example.test:8443"),
    "https://agent.example.test:8443"
)]
#[case::one_trailing_slash(Some("https://agent.example.test/"), "https://agent.example.test")]
#[case::every_trailing_slash(Some("https://agent.example.test//"), "https://agent.example.test")]
#[case::a_lone_scheme(Some("https://"), "https://https:")]
#[case::surrounding_spaces_kept(Some(" agent.example.test "), "https:// agent.example.test ")]
fn resolves_the_agent_api_base(#[case] configured: Option<&str>, #[case] expected: &str) {
    assert_eq!(api_base_from_text(configured), expected);
}

#[test]
fn the_public_base_follows_the_process_environment() {
    let environment = process_environment();
    assert_eq!(
        resolve_tinyfish_api_base(),
        api_base_from_text(environment.get(API_BASE_VARIABLE).map(String::as_str))
    );
}
