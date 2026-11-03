use super::{error::SpanError, sanitize_event_metadata::sanitize_event_metadata};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct ServiceSpanData {
    pub service_name: String,
    pub call_type: Option<String>,
    pub caller: Option<String>,
    pub error: Option<SpanError>,
    pub event_metadata: BTreeMap<String, String>,
}

impl ServiceSpanData {
    pub fn from_payload(
        service_name: String,
        call_type: Option<String>,
        caller: Option<String>,
        error: Option<String>,
        metadata: &BTreeMap<String, Value>,
    ) -> Self {
        let error = error.map(|message| SpanError {
            message: Some(message),
            error_type: None,
            code: None,
            stack_trace: None,
            llm_provider: None,
        });
        Self {
            service_name,
            call_type,
            caller,
            error,
            event_metadata: sanitize_event_metadata(metadata),
        }
    }
}

#[path = "service/tests.rs"]
mod tests;
