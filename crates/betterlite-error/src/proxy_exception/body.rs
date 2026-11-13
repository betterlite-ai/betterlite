use std::collections::BTreeMap;

use betterlite_redact::structured::Payload;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProxyErrorBody {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub param: Option<String>,
    pub code: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_specific_fields: BTreeMap<String, Payload>,
}
