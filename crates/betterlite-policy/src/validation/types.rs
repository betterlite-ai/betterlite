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
