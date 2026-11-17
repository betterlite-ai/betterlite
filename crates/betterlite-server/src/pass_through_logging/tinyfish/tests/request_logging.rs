#![cfg(test)]

use rstest::rstest;

use super::should_log_tinyfish_request;

#[rstest]
#[case::run_submit("POST", "https://agent.tinyfish.ai/v1/automation/run", true)]
#[case::run_async_submit("POST", "https://agent.tinyfish.ai/v1/automation/run-async", true)]
#[case::run_sse_submit("POST", "https://agent.tinyfish.ai/v1/automation/run-sse", true)]
#[case::a_relative_route("POST", "/v1/automation/run", true)]
#[case::a_route_with_a_query("POST", "https://agent.tinyfish.ai/v1/automation/run?x=1", true)]
#[case::a_prefixed_route("POST", "https://proxy.test/team/v1/automation/run", true)]
#[case::no_run_status_poll("GET", "https://agent.tinyfish.ai/v1/automation/run", false)]
#[case::no_cancel("POST", "https://agent.tinyfish.ai/v1/runs/run-1/cancel", false)]
#[case::no_automation_root("POST", "https://agent.tinyfish.ai/v1/automation", false)]
#[case::no_lookalike_segment("POST", "https://agent.tinyfish.ai/v1/automationX/run", false)]
#[case::no_lowercase_method("post", "https://agent.tinyfish.ai/v1/automation/run", false)]
#[case::no_empty_url("POST", "", false)]
#[case::no_unparsable_url("POST", "https://[::1/v1/automation/run", false)]
fn bills_only_run_submissions(#[case] method: &str, #[case] url_route: &str, #[case] billed: bool) {
    assert_eq!(should_log_tinyfish_request(method, url_route), billed);
}
