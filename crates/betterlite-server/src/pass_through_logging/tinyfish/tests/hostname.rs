#![cfg(test)]

use rstest::rstest;

use super::is_tinyfish_agent_url;

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
