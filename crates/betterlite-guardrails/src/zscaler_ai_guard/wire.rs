use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Direction {
    #[serde(rename = "IN")]
    In,
    #[serde(rename = "OUT")]
    Out,
}

#[derive(Debug, Serialize)]
pub struct PolicyRequest<'request> {
    pub direction: Direction,
    pub content: &'request str,
    #[serde(rename = "policyId", skip_serializing_if = "Option::is_none")]
    pub policy_id: Option<GuardrailValue>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PolicyResponse {
    #[serde(rename = "statusCode")]
    pub status_code: Option<GuardrailValue>,
    pub action: Option<GuardrailValue>,
    #[serde(rename = "errorMsg")]
    pub error_message: Option<GuardrailValue>,
    #[serde(rename = "transactionId")]
    pub transaction_id: Option<GuardrailValue>,
    #[serde(rename = "detectorResponses", default)]
    pub detector_responses: IndexMap<String, GuardrailValue>,
}
