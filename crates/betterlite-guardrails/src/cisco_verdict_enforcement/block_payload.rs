use serde::Serialize;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_verdict_enforcement::support::insert_serialized;

#[derive(Debug, Clone, Serialize)]
pub struct BlockPayload {
    pub error: &'static str,
    pub message: &'static str,
    pub provider: String,
    pub guardrail: String,
    pub surface: &'static str,
    pub direction: &'static str,
    pub action: &'static str,
    pub classifications: Vec<String>,
    pub severity: Option<String>,
    pub rules: Vec<Option<String>>,
    pub explanation: Option<String>,
    pub event_id: Option<String>,
}

// The HTTP-exception-like shape _http_exception_to_error_obj normalizes.
pub struct GuardrailException {
    pub status_code: u16,
    pub detail: Json,
}

pub(crate) fn error_fields_from_detail(detail: &Json) -> Fields {
    match detail {
        Json::Object(fields) => fields.clone(),
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            let mut fields = Fields::new();
            insert_serialized(&mut fields, "message", &other.python_str());
            fields
        }
    }
}

mod tests;
