use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyValidationErrorType {
    InvalidGuardrail,
    InvalidTeam,
    InvalidKey,
    InvalidModel,
    InvalidInheritance,
    CircularInheritance,
    InvalidScope,
    InvalidSyntax,
}

impl fmt::Display for PolicyValidationErrorType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let wire_value = match self {
            Self::InvalidGuardrail => "invalid_guardrail",
            Self::InvalidTeam => "invalid_team",
            Self::InvalidKey => "invalid_key",
            Self::InvalidModel => "invalid_model",
            Self::InvalidInheritance => "invalid_inheritance",
            Self::CircularInheritance => "circular_inheritance",
            Self::InvalidScope => "invalid_scope",
            Self::InvalidSyntax => "invalid_syntax",
        };
        formatter.write_str(wire_value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyValidationError {
    pub policy_name: String,
    pub error_type: PolicyValidationErrorType,
    pub message: String,
    pub field: Option<String>,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyValidationResponse {
    pub valid: bool,
    #[serde(default)]
    pub errors: Vec<PolicyValidationError>,
    #[serde(default)]
    pub warnings: Vec<PolicyValidationError>,
}
