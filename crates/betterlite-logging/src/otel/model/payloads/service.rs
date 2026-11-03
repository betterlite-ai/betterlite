use super::super::utils::coercion::OtelModelValue;
use super::{error::SpanError, sanitize_event_metadata::sanitize_event_metadata};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceSpanData {
    pub service_name: String,
    pub call_type: Option<String>,
    pub caller: Option<String>,
    pub error: Option<SpanError>,
    pub event_metadata: BTreeMap<String, String>,
}

impl ServiceSpanData {
    #[must_use]
    pub fn from_payload(
        service_name: String,
        call_type: Option<String>,
        caller: Option<String>,
        error_message: Option<String>,
        metadata: &BTreeMap<String, OtelModelValue>,
    ) -> Self {
        let error_data = error_message.map(|message| SpanError {
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
            error: error_data,
            event_metadata: sanitize_event_metadata(metadata),
        }
    }
}

#[path = "service/tests.rs"]
mod tests;
