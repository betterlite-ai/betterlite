use crate::otel::model::{
    GuardrailSpanData, LlmCallSpanData, McpListToolsSpanData, McpToolCallSpanData,
    ProxyRequestSpanData, ServiceSpanData,
};

#[must_use]
pub fn llm_call_span_name(data: &LlmCallSpanData) -> String {
    format!(
        "{} {}",
        data.operation.value(),
        data.request_model.as_deref().unwrap_or("")
    )
    .trim()
    .to_owned()
}
#[must_use]
pub fn mcp_tool_call_span_name(data: &McpToolCallSpanData) -> String {
    format!("{} {}", data.method.value(), data.tool_name)
        .trim()
        .to_owned()
}
#[must_use]
pub fn mcp_list_tools_span_name(data: &McpListToolsSpanData) -> String {
    data.method.value().to_owned()
}
#[must_use]
pub fn proxy_request_span_name(data: &ProxyRequestSpanData) -> String {
    format!("{} {}", data.http_method, data.route)
        .trim()
        .to_owned()
}
#[must_use]
pub fn guardrail_span_name(data: &GuardrailSpanData) -> String {
    format!("execute_guardrail {}", data.guardrail_name)
        .trim()
        .to_owned()
}
#[must_use]
pub fn service_span_name(data: &ServiceSpanData) -> String {
    format!(
        "{} {}",
        data.service_name,
        data.call_type.as_deref().unwrap_or("")
    )
    .trim()
    .to_owned()
}

#[cfg(test)]
#[path = "span_names/tests.rs"]
mod tests;
