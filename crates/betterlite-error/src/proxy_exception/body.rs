use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProxyErrorBody {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub param: Option<String>,
    pub code: String,
    #[serde(skip_serializing_if = "provider_fields_absent")]
    pub provider_specific_fields: Option<Map<String, Value>>,
}

fn provider_fields_absent(fields: &Option<Map<String, Value>>) -> bool {
    match fields {
        Some(map) => map.is_empty(),
        None => true,
    }
}
