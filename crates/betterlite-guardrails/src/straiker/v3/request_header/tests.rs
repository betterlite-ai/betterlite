#![cfg(test)]

use rstest::rstest;

use super::inbound_header;
use crate::straiker::v3::fixtures::object;

#[rstest]
#[case::matches_name_in_any_case(
    r#"{"proxy_server_request":{"headers":{"User-Agent":"claude-cli/1"}}}"#,
    "user-agent",
    Some("claude-cli/1")
)]
#[case::trims_value(
    r#"{"proxy_server_request":{"headers":{"x-a":"  v  "}}}"#,
    "x-a",
    Some("v")
)]
#[case::skips_blank_value_for_next_match(
    r#"{"proxy_server_request":{"headers":{"X-A":"  ","x-a":"v"}}}"#,
    "x-a",
    Some("v")
)]
#[case::ignores_non_text_value(r#"{"proxy_server_request":{"headers":{"x-a":1}}}"#, "x-a", None)]
#[case::other_header_only(r#"{"proxy_server_request":{"headers":{"x-b":"v"}}}"#, "x-a", None)]
#[case::headers_not_an_object(r#"{"proxy_server_request":{"headers":["x-a"]}}"#, "x-a", None)]
#[case::no_proxy_request(r"{}", "x-a", None)]
fn reads_header(#[case] source: &str, #[case] name: &str, #[case] expected: Option<&str>) {
    assert_eq!(inbound_header(&object(source), name).as_deref(), expected);
}
