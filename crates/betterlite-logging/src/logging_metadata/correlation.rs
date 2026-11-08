use std::collections::HashMap;

use betterlite_models::decisions::Json;

use crate::logging_metadata::params::LitellmParams;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceService {
    Langfuse,
}

impl TraceService {
    const fn cache_prefix(self) -> &'static str {
        match self {
            Self::Langfuse => "langfuse",
        }
    }
}

#[derive(Debug, Default)]
pub struct TraceIdCache {
    entries: HashMap<String, String>,
}

impl TraceIdCache {
    pub fn record(&mut self, call_id: &str, service: TraceService, trace_id: String) {
        self.entries
            .insert(format!("{}:{call_id}", service.cache_prefix()), trace_id);
    }

    #[must_use]
    pub fn logged_trace_id(&self, call_id: &str, service: TraceService) -> Option<&str> {
        self.entries
            .get(&format!("{}:{call_id}", service.cache_prefix()))
            .map(String::as_str)
    }
}

#[must_use]
pub fn logging_payload_trace_id(
    params: &LitellmParams,
    fallback_trace_id: &str,
    request_correlation_in_logs: bool,
) -> String {
    let metadata_trace_id = metadata_text(params, "trace_id");
    let metadata_session_id = metadata_text(params, "session_id");
    let candidates: [Option<&str>; 4] = if request_correlation_in_logs {
        [
            params.litellm_trace_id.as_deref(),
            params.litellm_session_id.as_deref(),
            metadata_trace_id.as_deref(),
            metadata_session_id.as_deref(),
        ]
    } else {
        [
            params.litellm_session_id.as_deref(),
            params.litellm_trace_id.as_deref(),
            metadata_session_id.as_deref(),
            metadata_trace_id.as_deref(),
        ]
    };
    let Some(candidate) = candidates.into_iter().flatten().next() else {
        return fallback_trace_id.to_owned();
    };
    candidate.to_owned()
}

#[must_use]
pub fn logging_payload_session_id(
    params: &LitellmParams,
    fallback_session_id: &str,
    request_correlation_in_logs: bool,
) -> String {
    if !request_correlation_in_logs {
        return String::new();
    }
    if let Some(session_id) = params.litellm_session_id.as_deref() {
        return session_id.to_owned();
    }
    if let Some(session_id) = metadata_text(params, "session_id") {
        return session_id;
    }
    fallback_session_id.to_owned()
}

fn metadata_text(params: &LitellmParams, key: &str) -> Option<String> {
    match params.metadata.as_ref()?.get(key)? {
        Json::Text(text) => Some(text.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

mod tests;
