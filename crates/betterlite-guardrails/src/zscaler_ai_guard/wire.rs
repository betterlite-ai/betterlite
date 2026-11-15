use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

// serde_json's arbitrary_precision feature hands a JSON float to an untagged enum as this one-key map
const NUMBER_KEY: &str = "$serde_json::private::Number";

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

impl PolicyResponse {
    // LiteLLM compares statusCode and the action and prints errorMsg and transactionId as Python holds them
    pub(crate) fn exact(mut self) -> Self {
        self.status_code = self.status_code.map(exact_value);
        self.action = self.action.map(exact_value);
        self.error_message = self.error_message.map(exact_value);
        self.transaction_id = self.transaction_id.map(exact_value);
        self
    }
}

pub(crate) fn exact_value(value: GuardrailValue) -> GuardrailValue {
    match value {
        GuardrailValue::Array(items) => {
            GuardrailValue::Array(items.into_iter().map(exact_value).collect())
        }
        GuardrailValue::Object(fields) => exact_object(fields),
        scalar @ (GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)) => scalar,
    }
}

fn exact_object(fields: BTreeMap<String, GuardrailValue>) -> GuardrailValue {
    if fields.len() == 1
        && let Some(GuardrailValue::Text(digits)) = fields.get(NUMBER_KEY)
        && let Some(number) = number_from_text(digits)
    {
        return number;
    }
    GuardrailValue::Object(
        fields
            .into_iter()
            .map(|(name, value)| (name, exact_value(value)))
            .collect(),
    )
}

fn number_from_text(text: &str) -> Option<GuardrailValue> {
    text.parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
        .map(GuardrailValue::Decimal)
}
