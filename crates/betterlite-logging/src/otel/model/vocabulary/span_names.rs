use crate::otel::vocabulary::constants::{GenAIOperation, MCPMethod};

#[path = "span_names/tests.rs"]
mod tests;

// the FastAPI instrumentor names the root server span; nothing here creates it
pub const LITELLM_PROXY_REQUEST_SPAN_NAME: &str = "Received Proxy Server Request";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedSpan<'name> {
    LlmCall {
        operation: GenAIOperation,
        request_model: &'name str,
    },
    McpToolCall {
        method: MCPMethod,
        tool_name: &'name str,
    },
    McpListTools {
        method: MCPMethod,
    },
    ProxyRequest {
        http_method: &'name str,
        route: &'name str,
    },
    Guardrail {
        guardrail_name: &'name str,
    },
    Service {
        service_name: &'name str,
        call_type: &'name str,
    },
}

impl NamedSpan<'_> {
    #[must_use]
    pub fn name(&self) -> String {
        let name = match *self {
            Self::LlmCall {
                operation,
                request_model,
            } => format!("{operation} {request_model}"),
            Self::McpToolCall { method, tool_name } => format!("{method} {tool_name}"),
            Self::McpListTools { method } => return method.to_string(),
            Self::ProxyRequest { http_method, route } => format!("{http_method} {route}"),
            Self::Guardrail { guardrail_name } => format!("execute_guardrail {guardrail_name}"),
            Self::Service {
                service_name,
                call_type,
            } => format!("{service_name} {call_type}"),
        };
        name.trim_matches(is_python_space).to_owned()
    }
}

// Python's str.strip also drops the separators U+001C to U+001F, which Rust does not count as whitespace
pub(crate) const fn is_python_space(character: char) -> bool {
    match character {
        '\u{1c}'..='\u{1f}' => true,
        other => other.is_whitespace(),
    }
}
