use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::future::Future;

use serde::Serialize;

use super::json_value::McpValue;
use crate::generic_guardrail_contracts::GuardrailInputType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailInputs {
    pub tools: Vec<ToolDefinition>,
    pub texts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: BTreeMap<String, McpValue>,
    pub strict: bool,
}

pub trait ApplyGuardrail {
    type Error: StdError + 'static;

    fn apply_guardrail(
        &self,
        inputs: GuardrailInputs,
        input_type: GuardrailInputType,
    ) -> impl Future<Output = Result<Option<Vec<String>>, Self::Error>> + Send;
}
