#![cfg(test)]

use rstest::rstest;

use super::headers;
use crate::straiker::v3::fixtures::{as_json, object, parsed};

const CLAUDE_AGENT: &str =
    r#"{"proxy_server_request":{"headers":{"user-agent":"claude-cli/2.1.0"}}}"#;

#[rstest]
#[case::nothing(r"{}", None, None, None, r"{}")]
#[case::session_header_forwarded(
    r#"{"proxy_server_request":{"headers":{"X-Claude-Code-Session-Id":" s-1 "}}}"#,
    None,
    None,
    None,
    r#"{"x-claude-code-session-id":"s-1"}"#
)]
#[case::route_values_only(
    r"{}",
    Some("app"),
    Some("sdk"),
    Some("openai.chat"),
    r#"{"x-s6r-agent":"app","x-s6r-client":"sdk","x-s6r-format":"openai.chat"}"#
)]
#[case::user_agent_names_agent_and_client(
    CLAUDE_AGENT,
    None,
    None,
    None,
    r#"{"x-s6r-agent":"Claude (LiteLLM)","x-s6r-client":"claude"}"#
)]
#[case::route_values_beat_user_agent(
    CLAUDE_AGENT,
    Some("pinned"),
    Some("sdk"),
    None,
    r#"{"x-s6r-agent":"pinned","x-s6r-client":"sdk"}"#
)]
#[case::caller_agent_header_beats_user_agent(
    r#"{"proxy_server_request":{"headers":{"x-s6r-agent":"caller","user-agent":"claude-cli/1"}}}"#,
    None,
    None,
    None,
    r#"{"x-s6r-agent":"caller","x-s6r-client":"claude"}"#
)]
#[case::route_agent_beats_caller_header(
    r#"{"proxy_server_request":{"headers":{"x-s6r-agent":"caller"}}}"#,
    Some("pinned"),
    None,
    None,
    r#"{"x-s6r-agent":"pinned"}"#
)]
#[case::empty_route_values_fall_through(
    CLAUDE_AGENT,
    Some(""),
    Some(""),
    Some(""),
    r#"{"x-s6r-agent":"Claude (LiteLLM)","x-s6r-client":"claude"}"#
)]
fn composes_headers(
    #[case] source: &str,
    #[case] agent_ref: Option<&str>,
    #[case] client: Option<&str>,
    #[case] format_hint: Option<&str>,
    #[case] expected: &str,
) {
    let composed = headers(&object(source), agent_ref, client, format_hint);
    assert_eq!(as_json(&composed), parsed(expected));
    assert_eq!(
        composed.keys().copied().collect::<Vec<_>>(),
        object(expected)
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
}
