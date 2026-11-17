#![cfg(test)]

use betterlite_config::loader::startup::process_environment;
use rstest::rstest;

use super::is_tinyfish_agent_url;
use super::settings::API_BASE_VARIABLE;
use super::support::rerun_test_with_environment;

const CHILD_MARKER: &str = "B3648_HOSTNAME_CHILD";
const OVERRIDE: &str = "override.example.test";

#[rstest]
#[case::the_agent_base("https://agent.tinyfish.ai/v1/automation/run", true)]
#[case::the_host_in_another_case("https://AGENT.TINYFISH.AI/v1/runs/run-1", true)]
#[case::another_port("https://agent.tinyfish.ai:8443/v1/runs/run-1", true)]
#[case::a_user_info_prefix("https://user@agent.tinyfish.ai/v1/runs/run-1", true)]
#[case::a_scheme_relative_url("//agent.tinyfish.ai/v1/runs/run-1", true)]
#[case::a_lookalike_host("https://agent.tinyfish.ai.evil.test/v1/runs", false)]
#[case::the_host_inside_the_path("https://evil.test/agent.tinyfish.ai", false)]
#[case::a_bare_host("agent.tinyfish.ai/v1/runs", false)]
#[case::plain_text("not a url", false)]
#[case::an_empty_url("", false)]
#[case::an_unparsable_url("https://[::1/v1/runs", false)]
fn matches_the_agent_host(#[case] url: &str, #[case] expected: bool) {
    assert_eq!(is_tinyfish_agent_url(url), expected);
}

// The resolved base reaches the host check through the process environment, so the child sets it.
#[test]
fn matches_the_base_configured_in_the_process_environment() {
    if process_environment().contains_key(CHILD_MARKER) {
        assert!(is_tinyfish_agent_url(
            "https://override.example.test/v1/runs/run-1"
        ));
        assert!(!is_tinyfish_agent_url(
            "https://agent.tinyfish.ai/v1/runs/run-1"
        ));
        println!("{CHILD_MARKER}");
        return;
    }
    let this_test = module_path!()
        .split_once("::")
        .map(|(_crate_name, path)| {
            format!("{path}::matches_the_base_configured_in_the_process_environment")
        })
        .expect("the module path has a crate prefix");
    rerun_test_with_environment(&this_test, API_BASE_VARIABLE, OVERRIDE, CHILD_MARKER);
}
