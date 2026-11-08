#![cfg(test)]

use super::*;
use crate::otel::model::metadata::wire::decode;

fn payload(text: &str) -> Payload {
    decode::<Payload>(text).expect("payload")
}

fn tool_call_payload() -> Payload {
    payload(
        r#"{
            "id": "call-1",
            "call_type": "call_mcp_tool",
            "response_cost": 0.002,
            "metadata": {
                "mcp_tool_call_metadata": {
                    "name": "get_weather",
                    "mcp_server_name": "weather",
                    "mcp_server_resource": "https://weather.example:8443",
                    "mcp_session_id": "session-1",
                    "arguments": {"city": "Paris"},
                    "result": {"temperature": 20}
                },
                "user_api_key_request_route": "/mcp"
            }
        }"#,
    )
}

#[test]
fn tool_call_span_matches_litellm() {
    let span = MCPToolCallSpanData::from_standard_logging_payload(&tool_call_payload(), false)
        .expect("span");
    assert_eq!(span.operation, GenAIOperation::ExecuteTool);
    assert_eq!(span.method, MCPMethod::ToolsCall);
    assert_eq!(span.tool_name, "get_weather");
    assert_eq!(span.server_name.as_deref(), Some("weather"));
    assert_eq!(span.server_address.as_deref(), Some("weather.example"));
    assert_eq!(span.server_port, Some(8443));
    assert_eq!(span.session_id.as_deref(), Some("session-1"));
    assert_eq!(span.arguments_json, None);
    assert_eq!(span.result_json, None);
    assert_eq!(span.error, None);
    assert_eq!(span.response_cost, Some(Decimal::new(2, 3)));
    assert_eq!(span.identity.call_id.as_deref(), Some("call-1"));
    assert_eq!(span.identity.request_route.as_deref(), Some("/mcp"));
}

#[test]
fn tool_call_content_follows_capture() {
    let span = MCPToolCallSpanData::from_standard_logging_payload(&tool_call_payload(), true)
        .expect("span");
    assert_eq!(span.arguments_json.as_deref(), Some(r#"{"city":"Paris"}"#));
    assert_eq!(span.result_json.as_deref(), Some(r#"{"temperature":20}"#));
}

#[rstest::rstest]
#[case::string_arguments(r#"{"name":"tool","arguments":"raw text"}"#, Some("raw text"))]
#[case::null_arguments(r#"{"name":"tool","arguments":null}"#, None)]
fn arguments_pass_through_or_stay_absent(#[case] metadata: &str, #[case] expected: Option<&str>) {
    let raw = payload(&format!(
        r#"{{"metadata": {{"mcp_tool_call_metadata": {metadata}}}}}"#
    ));
    let span = MCPToolCallSpanData::from_standard_logging_payload(&raw, true).expect("span");
    assert_eq!(span.arguments_json.as_deref(), expected);
    assert_eq!(span.result_json, None);
}

#[test]
fn a_payload_without_tool_metadata_reads_as_empty() {
    let raw = payload(
        r#"{"call_type": "call_mcp_tool", "metadata": {"mcp_tool_call_metadata": "gone"}}"#,
    );
    let span = MCPToolCallSpanData::from_standard_logging_payload(&raw, true).expect("span");
    assert_eq!(span.tool_name, "");
    assert_eq!(span.server_name, None);
    assert_eq!(span.server_address, None);
    assert_eq!(span.server_port, None);
    assert_eq!(span.session_id, None);
    assert_eq!(span.arguments_json, None);
    assert_eq!(span.result_json, None);
}

#[test]
fn a_failed_tool_call_carries_the_span_error() {
    let raw = payload(
        r#"{
            "status": "failure",
            "error_str": "tool exploded",
            "error_information": {"error_class": "ToolError", "error_code": "502"},
            "metadata": {"mcp_tool_call_metadata": {"name": "get_weather"}}
        }"#,
    );
    let span = MCPToolCallSpanData::from_standard_logging_payload(&raw, false).expect("span");
    let error = span.error.expect("span error");
    assert_eq!(error.error_type.as_deref(), Some("ToolError"));
    assert_eq!(error.message.as_deref(), Some("tool exploded"));
    assert_eq!(error.code.as_deref(), Some("502"));
}

#[rstest::rstest]
#[case::https_default("https://weather.example", Some("weather.example"), Some(443))]
#[case::http_default("http://weather.example/v1", Some("weather.example"), Some(80))]
#[case::explicit_port("http://weather.example:8080", Some("weather.example"), Some(8080))]
#[case::port_zero_takes_the_default(
    "https://weather.example:0",
    Some("weather.example"),
    Some(443)
)]
#[case::other_scheme("ftp://weather.example", Some("weather.example"), None)]
#[case::other_scheme_port_zero("ftp://weather.example:0", Some("weather.example"), None)]
#[case::uppercase_scheme_and_host("HTTPS://Weather.Example", Some("weather.example"), Some(443))]
#[case::userinfo(
    "https://user:pass@weather.example:9000/x",
    Some("weather.example"),
    Some(9000)
)]
#[case::bracketed_ipv6("https://[2001:db8::1]:9443/x", Some("2001:db8::1"), Some(9443))]
#[case::bracketed_ipv6_default("http://[2001:db8::1]", Some("2001:db8::1"), Some(80))]
#[case::percent_escaped_host("http://foo%2F.example:8080", Some("foo%2F.example"), Some(8080))]
#[case::bracketed_zone_host("http://[fe80::1%25ETH0]:8080", Some("fe80::1%25ETH0"), Some(8080))]
#[case::zone_scoped_without_brackets("http://fe80::1%25eth0", None, None)]
#[case::port_too_large("https://weather.example:70000", None, None)]
#[case::port_not_a_number("https://weather.example:http", None, None)]
#[case::signed_port("https://weather.example:+80", None, None)]
#[case::unmatched_bracket("http://[2001:db8::1", None, None)]
#[case::unopened_bracket("http://2001:db8::1]", None, None)]
#[case::empty_host("https://:8080", None, None)]
#[case::no_scheme("weather.example:443", None, None)]
#[case::empty("", None, None)]
fn upstream_origin_splits_like_urlsplit(
    #[case] resource: &str,
    #[case] address: Option<&str>,
    #[case] port: Option<u16>,
) {
    assert_eq!(
        upstream_address_port(Some(resource)),
        (address.map(str::to_owned), port)
    );
}

#[test]
fn an_absent_origin_gives_no_address_or_port() {
    assert_eq!(upstream_address_port(None), (None, None));
}

#[test]
fn list_tools_span_matches_litellm() {
    let raw = payload(
        r#"{"call_type": "list_mcp_tools", "id": "call-2", "metadata": {"user_api_key_request_route": "/mcp"}}"#,
    );
    let span = MCPListToolsSpanData::from_standard_logging_payload(&raw).expect("span");
    assert_eq!(span.method, MCPMethod::ToolsList);
    assert_eq!(span.session_id, None);
    assert_eq!(span.error, None);
    assert_eq!(span.identity.call_id.as_deref(), Some("call-2"));
    assert_eq!(span.identity.request_route.as_deref(), Some("/mcp"));
}

#[test]
fn a_failed_list_tools_call_carries_the_span_error() {
    let raw = payload(
        r#"{
            "call_type": "list_mcp_tools",
            "status": "failure",
            "error_information": {"error_class": "MCPError", "error_code": "503"}
        }"#,
    );
    let span = MCPListToolsSpanData::from_standard_logging_payload(&raw).expect("span");
    let error = span.error.expect("span error");
    assert_eq!(error.error_type.as_deref(), Some("MCPError"));
    assert_eq!(error.code.as_deref(), Some("503"));
}

#[rstest::rstest]
#[case::metadata_wins(
    r#"{"call_type": "completion", "metadata": {"mcp_tool_call_metadata": {"name": "tool"}}}"#,
    PayloadKind::McpToolCall
)]
#[case::call_type(r#"{"call_type": "call_mcp_tool"}"#, PayloadKind::McpToolCall)]
#[case::list_tools(r#"{"call_type": "list_mcp_tools"}"#, PayloadKind::McpListTools)]
#[case::empty_metadata(
    r#"{"call_type": "completion", "metadata": {"mcp_tool_call_metadata": {}}}"#,
    PayloadKind::Inference
)]
#[case::empty_metadata_list_tools(
    r#"{"call_type": "list_mcp_tools", "metadata": {"mcp_tool_call_metadata": {}}}"#,
    PayloadKind::McpListTools
)]
#[case::inference(r#"{"call_type": "completion"}"#, PayloadKind::Inference)]
#[case::no_call_type(r#"{"model": "gpt-4o"}"#, PayloadKind::Inference)]
fn payload_kind_picks_the_span(#[case] raw: &str, #[case] expected: PayloadKind) {
    assert_eq!(PayloadKind::from_payload(&payload(raw)), expected);
}
