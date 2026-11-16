#![cfg(test)]

use rstest::rstest;

use super::client_from_user_agent;
use crate::straiker::v3::fixtures::object;

#[rstest]
#[case::claude_code(r#"{"proxy_server_request":{"headers":{"user-agent":"claude-cli/2.1.0 (external, cli)"}}}"#, Some(("claude", "Claude (LiteLLM)")))]
#[case::prefix_matches_in_any_case(r#"{"proxy_server_request":{"headers":{"User-Agent":"Claude-CLI/1"}}}"#, Some(("claude", "Claude (LiteLLM)")))]
#[case::prefix_must_start_the_value(
    r#"{"proxy_server_request":{"headers":{"user-agent":"curl claude-cli/1"}}}"#,
    None
)]
#[case::unrecognised_agent(
    r#"{"proxy_server_request":{"headers":{"user-agent":"python-httpx/0.27"}}}"#,
    None
)]
#[case::blank_agent(r#"{"proxy_server_request":{"headers":{"user-agent":"  "}}}"#, None)]
#[case::no_headers(r"{}", None)]
fn recognises_client(#[case] source: &str, #[case] expected: Option<(&str, &str)>) {
    let found = client_from_user_agent(&object(source));
    let found = found
        .as_ref()
        .map(|(client, agent)| (client.as_str(), agent.as_str()));
    assert_eq!(found, expected);
}
