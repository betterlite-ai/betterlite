use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestCustomCodeResponse {
    pub success: bool,
    pub result: Option<BTreeMap<String, GuardrailValue>>,
    pub error: Option<String>,
    pub error_type: Option<ErrorType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    Compilation,
    Execution,
}

impl TestCustomCodeResponse {
    #[must_use]
    pub const fn failure(error: String, error_type: ErrorType) -> Self {
        Self {
            success: false,
            result: None,
            error: Some(error),
            error_type: Some(error_type),
        }
    }

    #[must_use]
    pub const fn success(result: BTreeMap<String, GuardrailValue>) -> Self {
        Self {
            success: true,
            result: Some(result),
            error: None,
            error_type: None,
        }
    }
}
