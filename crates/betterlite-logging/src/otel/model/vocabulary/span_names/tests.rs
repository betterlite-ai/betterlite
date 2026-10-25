#![cfg(test)]

use rstest::rstest;

use crate::otel::vocabulary::constants::{GenAIOperation, MCPMethod};
use crate::otel::vocabulary::span_names::NamedSpan;

#[rstest]
#[case::llm(NamedSpan::LlmCall { operation: GenAIOperation::Embeddings, request_model: "text-embedding-3" }, "embeddings text-embedding-3")]
#[case::llm_without_model(NamedSpan::LlmCall { operation: GenAIOperation::Chat, request_model: "" }, "chat")]
#[case::tool(NamedSpan::McpToolCall { method: MCPMethod::ToolsCall, tool_name: "get-weather" }, "tools/call get-weather")]
#[case::tool_without_name(NamedSpan::McpToolCall { method: MCPMethod::ToolsCall, tool_name: "" }, "tools/call")]
#[case::list_tools(NamedSpan::McpListTools { method: MCPMethod::ToolsList }, "tools/list")]
#[case::proxy(NamedSpan::ProxyRequest { http_method: "POST", route: "/chat/completions" }, "POST /chat/completions")]
#[case::guardrail(NamedSpan::Guardrail { guardrail_name: "pii" }, "execute_guardrail pii")]
#[case::guardrail_without_name(NamedSpan::Guardrail { guardrail_name: "" }, "execute_guardrail")]
#[case::service(NamedSpan::Service { service_name: "redis", call_type: "set" }, "redis set")]
#[case::service_without_call_type(NamedSpan::Service { service_name: "redis", call_type: "" }, "redis")]
#[case::python_strip(NamedSpan::Service { service_name: "\u{1f}redis", call_type: " \u{1c}" }, "redis")]
fn span_names_follow_the_conventions(#[case] span: NamedSpan<'_>, #[case] expected: &str) {
    assert_eq!(span.name(), expected);
}
