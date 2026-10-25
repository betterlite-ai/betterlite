#![cfg(test)]

use crate::otel::model::{
    GuardrailSpanData, LlmCallSpanData, McpListToolsSpanData, McpToolCallSpanData,
    ProxyRequestSpanData, ServiceSpanData,
};
use crate::otel::vocabulary::*;

#[test]
fn span_names_follow_role_conventions_and_strip_empty_values() {
    assert_eq!(
        llm_call_span_name(&LlmCallSpanData {
            operation: GenAIOperation::CHAT,
            request_model: None
        }),
        "chat"
    );
    assert_eq!(
        mcp_tool_call_span_name(&McpToolCallSpanData {
            method: MCPMethod::TOOLS_CALL,
            tool_name: String::new()
        }),
        "tools/call"
    );
    assert_eq!(
        mcp_list_tools_span_name(&McpListToolsSpanData {
            method: MCPMethod::TOOLS_LIST
        }),
        "tools/list"
    );
    assert_eq!(
        proxy_request_span_name(&ProxyRequestSpanData {
            http_method: "GET".to_owned(),
            route: "/health".to_owned()
        }),
        "GET /health"
    );
    assert_eq!(
        guardrail_span_name(&GuardrailSpanData {
            guardrail_name: "pii".to_owned()
        }),
        "execute_guardrail pii"
    );
    assert_eq!(
        service_span_name(&ServiceSpanData {
            service_name: "redis".to_owned(),
            call_type: None
        }),
        "redis"
    );
}
