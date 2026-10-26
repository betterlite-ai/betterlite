use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum McpGuardrailError {
    #[error("content blocked: {reason}")]
    Blocked { reason: String },
    #[error("guardrail returned {returned} texts for {expected} MCP strings")]
    TextCount { returned: usize, expected: usize },
    #[error("guardrails produced conflicting rewrites at {path}")]
    ConflictingRewrite { path: String },
    #[error("guardrail failed: {0}")]
    Guardrail(String),
}

impl McpGuardrailError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        400
    }
}
