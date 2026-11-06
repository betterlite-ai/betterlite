use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

pub const SCAN_ENDPOINT: &str = "/xecguard/v1/scan";
pub const GROUNDING_ENDPOINT: &str = "/xecguard/v1/grounding";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanType {
    Input,
    Response,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XecGuardMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundingDocument {
    pub document_id: String,
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanPayload {
    pub model: String,
    pub scan_type: ScanType,
    pub messages: Vec<XecGuardMessage>,
    pub policy_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroundingPayload {
    pub model: String,
    pub prompt: String,
    pub response: String,
    pub documents: Vec<GroundingDocument>,
    pub strictness: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<GuardrailValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<GuardrailValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xecguard_result: Option<GuardrailValue>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, GuardrailValue>,
}

impl ScanResult {
    #[must_use]
    pub fn is_unsafe(&self) -> bool {
        match &self.decision {
            Some(GuardrailValue::Text(decision)) => decision == "UNSAFE",
            _ => false,
        }
    }

    #[must_use]
    pub fn trace_id_text(&self) -> &str {
        match &self.trace_id {
            Some(GuardrailValue::Text(trace_id)) => trace_id,
            _ => "",
        }
    }

    #[must_use]
    pub fn fields(&self) -> BTreeMap<String, GuardrailValue> {
        let mut fields = self.extra.clone();
        if let Some(decision) = &self.decision {
            fields.insert("decision".to_owned(), decision.clone());
        }
        if let Some(trace_id) = &self.trace_id {
            fields.insert("trace_id".to_owned(), trace_id.clone());
        }
        if let Some(result) = &self.xecguard_result {
            fields.insert("xecguard_result".to_owned(), result.clone());
        }
        fields
    }
}

mod tests;
