use super::super::utils::coercion::OtelModelValue;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpanError {
    pub error_type: Option<String>,
    pub message: Option<String>,
    pub code: Option<String>,
    pub stack_trace: Option<String>,
    pub llm_provider: Option<String>,
}

impl SpanError {
    #[must_use]
    pub fn from_payload(payload: &BTreeMap<String, OtelModelValue>) -> Option<Self> {
        if payload.get("status").and_then(OtelModelValue::as_str) != Some("failure") {
            return None;
        }
        let info = payload
            .get("error_information")
            .and_then(OtelModelValue::as_object);
        let message = info
            .and_then(|map| map.get("error_message"))
            .and_then(OtelModelValue::as_str)
            .or_else(|| payload.get("error_str").and_then(OtelModelValue::as_str));
        let error_type = info
            .and_then(|map| map.get("error_class"))
            .and_then(OtelModelValue::as_str)
            .or_else(|| {
                info.and_then(|map| map.get("error_code"))
                    .and_then(OtelModelValue::as_str)
            });
        Some(Self {
            error_type: error_type.map(str::to_owned),
            message: message.map(str::to_owned),
            code: info
                .and_then(|map| map.get("error_code"))
                .and_then(OtelModelValue::as_str)
                .map(str::to_owned),
            stack_trace: info
                .and_then(|map| map.get("traceback"))
                .and_then(OtelModelValue::as_str)
                .map(str::to_owned),
            llm_provider: info
                .and_then(|map| map.get("llm_provider"))
                .and_then(OtelModelValue::as_str)
                .map(str::to_owned),
        })
    }
}

#[path = "error/tests.rs"]
mod tests;
