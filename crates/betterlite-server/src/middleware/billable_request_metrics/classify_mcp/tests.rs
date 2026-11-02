#![cfg(test)]

use rstest::rstest;

use super::classify_mcp_route;

#[rstest]
#[case::bare_mcp("/mcp", Some("/mcp"))]
#[case::mcp_with_resource("/mcp/tools/list", Some("/mcp"))]
#[case::rest_tool_call("/mcp-rest/tools/call", Some("/mcp"))]
#[case::rest_tool_list_excluded("/mcp-rest/tools/list", None)]
#[case::dynamic_transport_single_segment("/my-agent/mcp", Some("/mcp"))]
#[case::dynamic_transport_toolset("/toolset/my-agent/mcp", Some("/mcp"))]
#[case::dynamic_transport_two_segments_excluded("/a/b/mcp", None)]
#[case::management_bare_excluded("/v1/mcp", None)]
#[case::management_deep_excluded("/v1/mcp/servers", None)]
#[case::unrelated_path("/health/liveliness", None)]
fn classifies_mcp_routes(#[case] path: &str, #[case] expected: Option<&str>) {
    assert_eq!(classify_mcp_route(path), expected);
}
