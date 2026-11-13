use serde::Serialize;

use super::status::DataDogStatus;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DatadogPayload {
    pub ddsource: String,
    pub ddtags: String,
    pub hostname: String,
    pub message: String,
    pub service: String,
    pub status: DataDogStatus,
    #[serde(rename = "dd.trace_id", skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(rename = "dd.span_id", skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
}
