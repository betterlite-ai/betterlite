use std::fmt::Display;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum HookRegistryError {
    #[error("support for the {kind} guardrail is not built yet")]
    NotBuilt { kind: String },
    // LiteLLM lets the initializer's own ValueError text reach the caller unchanged
    #[error("{message}")]
    Init { kind: String, message: String },
}

impl HookRegistryError {
    #[must_use]
    pub fn init<E>(kind: &str, error: E) -> Self
    where
        E: Display,
    {
        Self::Init {
            kind: kind.to_owned(),
            message: error.to_string(),
        }
    }

    // LiteLLM skips a config guardrail whose initializer raises ValueError, and nothing else
    #[must_use]
    pub const fn skips_startup(&self) -> bool {
        match self {
            Self::Init { .. } => true,
            Self::NotBuilt { .. } => false,
        }
    }
}
