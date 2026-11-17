#![cfg(test)]

use betterlite_config::loader::startup::process_environment;
use rstest::rstest;

use super::resolve_tinyfish_api_base;
use super::settings::{API_BASE_VARIABLE, api_base_from_text};
use super::support::rerun_test_with_environment;

const CHILD_MARKER: &str = "B3648_API_BASE_CHILD";
const OVERRIDE: &str = "override.example.test";

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

// The child run is the only way to prove the public resolver reads the process environment.
#[test]
fn the_public_base_follows_the_process_environment() {
    if process_environment().contains_key(CHILD_MARKER) {
        assert_eq!(resolve_tinyfish_api_base(), "https://override.example.test");
        println!("{CHILD_MARKER}");
        return;
    }
    let this_test = module_path!()
        .split_once("::")
        .map(|(_crate_name, path)| {
            format!("{path}::the_public_base_follows_the_process_environment")
        })
        .expect("the module path has a crate prefix");
    rerun_test_with_environment(&this_test, API_BASE_VARIABLE, OVERRIDE, CHILD_MARKER);
}
