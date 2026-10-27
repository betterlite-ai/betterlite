use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::future::Future;

use serde::Serialize;

use super::json_value::McpValue;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::generic_guardrail_contracts::headers::LoggingObjectContext;
use crate::mcp_guardrail_translation::tool_request::McpToolRequest;

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
        request_data: Option<&mut McpToolRequest>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut LoggingObjectContext>,
    ) -> impl Future<Output = Result<Option<Vec<String>>, Self::Error>> + Send;
}
