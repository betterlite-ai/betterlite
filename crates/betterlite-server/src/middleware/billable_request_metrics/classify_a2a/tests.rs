#![cfg(test)]

use rstest::rstest;

use super::classify_a2a_route;

#[rstest]
#[case::v1_prefix_message_send("/v1/a2a/agent-1/message/send", Some("/a2a"))]
#[case::bare_prefix_message_send("/a2a/agent-1/message/send", Some("/a2a"))]
#[case::bare_agent_without_suffix_not_billable("/a2a/agent-1", None)]
#[case::task_rpc_not_billable("/a2a/agent-1/tasks/get", None)]
#[case::message_stream_not_billable("/a2a/agent-1/message/stream", None)]
#[case::unrelated_path("/health/liveliness", None)]
fn classifies_a2a_routes(#[case] path: &str, #[case] expected: Option<&str>) {
    assert_eq!(classify_a2a_route(path), expected);
}
