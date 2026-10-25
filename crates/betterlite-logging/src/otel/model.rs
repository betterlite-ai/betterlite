use super::vocabulary::constants::{GenAIOperation, MCPMethod};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct LlmCallSpanData {
    pub operation: GenAIOperation,
    #[serde(default)]
    pub request_model: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct McpToolCallSpanData {
    pub method: MCPMethod,
    #[serde(default)]
    pub tool_name: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct McpListToolsSpanData {
    pub method: MCPMethod,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ProxyRequestSpanData {
    pub http_method: String,
    #[serde(default)]
    pub route: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GuardrailSpanData {
    #[serde(default)]
    pub guardrail_name: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ServiceSpanData {
    pub service_name: String,
    #[serde(default)]
    pub call_type: Option<String>,
}
