#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum SpanRole {
    ProxyRequest,
    LlmCall,
    McpToolCall,
    McpListTools,
    Guardrail,
    DbCall,
    Service,
}
impl SpanRole {
    pub const PROXY_REQUEST: Self = Self::ProxyRequest;
    pub const LLM_CALL: Self = Self::LlmCall;
    pub const MCP_TOOL_CALL: Self = Self::McpToolCall;
    pub const MCP_LIST_TOOLS: Self = Self::McpListTools;
    pub const GUARDRAIL: Self = Self::Guardrail;
    pub const DB_CALL: Self = Self::DbCall;
    pub const SERVICE: Self = Self::Service;
}
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum LiteLLMSpanKind {
    Server,
    Client,
    Internal,
    Producer,
    Consumer,
}
impl LiteLLMSpanKind {
    pub const SERVER: Self = Self::Server;
    pub const CLIENT: Self = Self::Client;
    pub const INTERNAL: Self = Self::Internal;
    pub const PRODUCER: Self = Self::Producer;
    pub const CONSUMER: Self = Self::Consumer;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanSpec {
    pub role: SpanRole,
    pub kind: LiteLLMSpanKind,
    pub parent: Option<SpanRole>,
}
pub static SPAN_REGISTRY: &[SpanSpec] = &[
    SpanSpec {
        role: SpanRole::PROXY_REQUEST,
        kind: LiteLLMSpanKind::SERVER,
        parent: None,
    },
    SpanSpec {
        role: SpanRole::LLM_CALL,
        kind: LiteLLMSpanKind::CLIENT,
        parent: Some(SpanRole::PROXY_REQUEST),
    },
    SpanSpec {
        role: SpanRole::MCP_TOOL_CALL,
        kind: LiteLLMSpanKind::CLIENT,
        parent: Some(SpanRole::PROXY_REQUEST),
    },
    SpanSpec {
        role: SpanRole::MCP_LIST_TOOLS,
        kind: LiteLLMSpanKind::CLIENT,
        parent: Some(SpanRole::PROXY_REQUEST),
    },
    SpanSpec {
        role: SpanRole::GUARDRAIL,
        kind: LiteLLMSpanKind::INTERNAL,
        parent: Some(SpanRole::PROXY_REQUEST),
    },
    SpanSpec {
        role: SpanRole::DB_CALL,
        kind: LiteLLMSpanKind::CLIENT,
        parent: Some(SpanRole::PROXY_REQUEST),
    },
    SpanSpec {
        role: SpanRole::SERVICE,
        kind: LiteLLMSpanKind::INTERNAL,
        parent: Some(SpanRole::PROXY_REQUEST),
    },
];
#[must_use]
pub fn root_roles() -> Vec<SpanRole> {
    SPAN_REGISTRY
        .iter()
        .filter(|spec| spec.parent.is_none())
        .map(|spec| spec.role)
        .collect()
}
#[must_use]
pub fn child_roles(parent: SpanRole) -> Vec<SpanRole> {
    SPAN_REGISTRY
        .iter()
        .filter(|spec| spec.parent == Some(parent))
        .map(|spec| spec.role)
        .collect()
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryError {
    MismatchedRole,
    UnknownParent,
    MissingRole,
}
pub fn validate_registry() -> Result<(), RegistryError> {
    let roles = [
        SpanRole::PROXY_REQUEST,
        SpanRole::LLM_CALL,
        SpanRole::MCP_TOOL_CALL,
        SpanRole::MCP_LIST_TOOLS,
        SpanRole::GUARDRAIL,
        SpanRole::DB_CALL,
        SpanRole::SERVICE,
    ];
    for spec in SPAN_REGISTRY {
        if !roles.contains(&spec.role) {
            return Err(RegistryError::MismatchedRole);
        }
        if let Some(parent) = spec.parent
            && !SPAN_REGISTRY
                .iter()
                .any(|candidate| candidate.role == parent)
        {
            return Err(RegistryError::UnknownParent);
        }
    }
    if roles
        .iter()
        .any(|role| !SPAN_REGISTRY.iter().any(|spec| spec.role == *role))
    {
        return Err(RegistryError::MissingRole);
    }
    Ok(())
}
#[cfg(test)]
#[path = "registry/tests.rs"]
mod tests;
