use strum::{Display, EnumIter, IntoEnumIterator as _};

#[path = "registry/tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum SpanRole {
    ProxyRequest,
    LlmCall,
    McpToolCall,
    McpListTools,
    Guardrail,
    DbCall,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[strum(serialize_all = "snake_case")]
pub enum LiteLLMSpanKind {
    Server,
    Client,
    Internal,
    Producer,
    Consumer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanSpec {
    pub role: SpanRole,
    pub kind: LiteLLMSpanKind,
    pub parent: Option<SpanRole>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("span role {role} is declared more than once")]
    DuplicateRole { role: SpanRole },
    #[error("span role {role} declares unknown parent {parent}")]
    UnknownParent { role: SpanRole, parent: SpanRole },
    #[error("the span registry is missing roles {roles:?}")]
    MissingRoles { roles: Vec<SpanRole> },
}

// MCP spans are CLIENT spans because the proxy is an MCP client of the upstream server
pub const SPAN_REGISTRY: [SpanSpec; 7] = [
    SpanSpec {
        role: SpanRole::ProxyRequest,
        kind: LiteLLMSpanKind::Server,
        parent: None,
    },
    SpanSpec {
        role: SpanRole::LlmCall,
        kind: LiteLLMSpanKind::Client,
        parent: Some(SpanRole::ProxyRequest),
    },
    SpanSpec {
        role: SpanRole::McpToolCall,
        kind: LiteLLMSpanKind::Client,
        parent: Some(SpanRole::ProxyRequest),
    },
    SpanSpec {
        role: SpanRole::McpListTools,
        kind: LiteLLMSpanKind::Client,
        parent: Some(SpanRole::ProxyRequest),
    },
    SpanSpec {
        role: SpanRole::Guardrail,
        kind: LiteLLMSpanKind::Internal,
        parent: Some(SpanRole::ProxyRequest),
    },
    SpanSpec {
        role: SpanRole::DbCall,
        kind: LiteLLMSpanKind::Client,
        parent: Some(SpanRole::ProxyRequest),
    },
    SpanSpec {
        role: SpanRole::Service,
        kind: LiteLLMSpanKind::Internal,
        parent: Some(SpanRole::ProxyRequest),
    },
];

pub fn validate_registry(registry: &[SpanSpec]) -> Result<(), RegistryError> {
    for spec in registry {
        if registry
            .iter()
            .filter(|other| other.role == spec.role)
            .count()
            > 1
        {
            return Err(RegistryError::DuplicateRole { role: spec.role });
        }
        if let Some(parent) = spec.parent
            && !registry.iter().any(|other| other.role == parent)
        {
            return Err(RegistryError::UnknownParent {
                role: spec.role,
                parent,
            });
        }
    }
    let missing: Vec<SpanRole> = SpanRole::iter()
        .filter(|role| !registry.iter().any(|spec| spec.role == *role))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(RegistryError::MissingRoles { roles: missing })
    }
}
