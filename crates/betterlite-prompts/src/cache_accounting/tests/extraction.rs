use rstest::rstest;

use crate::cache_accounting::extraction::{request_system, request_user_agent};
use crate::cache_accounting::tests::support::json;

#[rstest]
#[case::plain(
    r#"{"proxy_server_request": {"headers": {"user-agent": "claude-cli/2.0"}}}"#,
    Some("claude-cli/2.0")
)]
#[case::mixed_case(
    r#"{"proxy_server_request": {"headers": {"User-Agent": "claude-cli/2.0"}}}"#,
    Some("claude-cli/2.0")
)]
#[case::first_match_wins(
    r#"{"proxy_server_request": {"headers": {"USER-AGENT": "first", "user-agent": "second"}}}"#,
    Some("first")
)]
#[case::other_headers(
    r#"{"proxy_server_request": {"headers": {"x-api-key": "k", "user-agent": "ua"}}}"#,
    Some("ua")
)]
#[case::non_text_value(r#"{"proxy_server_request": {"headers": {"user-agent": 7}}}"#, None)]
#[case::missing_headers(r#"{"proxy_server_request": {}}"#, None)]
#[case::headers_not_an_object(r#"{"proxy_server_request": {"headers": "x"}}"#, None)]
#[case::proxy_request_not_an_object(r#"{"proxy_server_request": 5}"#, None)]
#[case::missing_proxy_request("{}", None)]
#[case::kwargs_not_an_object("[]", None)]
fn request_user_agent_reads_the_first_matching_header(
    #[case] request_kwargs: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(request_user_agent(&json(request_kwargs)), expected);
}

#[rstest]
#[case::text(r#"{"system": "a system prompt"}"#, Some(r#""a system prompt""#))]
#[case::block_list(
    r#"{"system": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}"#,
    Some(r#"[{"type":"text","text":"a"},{"type":"text","text":"b"}]"#)
)]
#[case::empty_list(r#"{"system": []}"#, Some("[]"))]
#[case::null(r#"{"system": null}"#, None)]
#[case::object(r#"{"system": {"text": "a"}}"#, None)]
#[case::number(r#"{"system": 5}"#, None)]
#[case::missing("{}", None)]
#[case::kwargs_not_an_object(r#""x""#, None)]
fn request_system_keeps_a_string_or_a_block_list(
    #[case] request_kwargs: &str,
    #[case] expected: Option<&str>,
) {
    let kwargs = json(request_kwargs);
    let system = request_system(&kwargs);
    let rendered = system.map(|value| serde_json::to_string(value).unwrap());
    assert_eq!(rendered.as_deref(), expected);
}
