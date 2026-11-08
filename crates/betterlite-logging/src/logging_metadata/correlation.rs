use std::time::{Duration, Instant};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

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

#[derive(Debug)]
pub struct TraceIdCache {
    capacity: usize,
    ttl: Duration,
    entries: IndexMap<String, (String, Instant)>, // value and the instant it was first stored
}

impl TraceIdCache {
    #[must_use]
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            capacity,
            ttl,
            entries: IndexMap::new(),
        }
    }

    // Expired entries go first, then the oldest, as InMemoryCache.evict_cache does; a live key keeps its expiry
    pub fn record(&mut self, call_id: &str, service: TraceService, trace_id: String, now: Instant) {
        self.entries
            .retain(|_, (_, stored)| now.saturating_duration_since(*stored) < self.ttl);
        let excess = self
            .entries
            .len()
            .saturating_sub(self.capacity.saturating_sub(1));
        for _ in 0..excess {
            self.entries.shift_remove_index(0);
        }
        let key = format!("{}:{call_id}", service.cache_prefix());
        let slot = self.entries.entry(key).or_insert((String::new(), now));
        slot.0 = trace_id;
    }

    #[must_use]
    pub fn logged_trace_id(
        &self,
        call_id: &str,
        service: TraceService,
        now: Instant,
    ) -> Option<&str> {
        let (value, stored) = self
            .entries
            .get(&format!("{}:{call_id}", service.cache_prefix()))?;
        (now.saturating_duration_since(*stored) <= self.ttl).then_some(value.as_str())
    }
}

#[must_use]
pub fn logging_payload_trace_id(
    params: &LitellmParams,
    fallback_trace_id: &str,
    request_correlation_in_logs: bool,
) -> String {
    let own_trace = params.litellm_trace_id.clone();
    let own_session = params.litellm_session_id.clone();
    let meta_trace = metadata_text(params, "trace_id");
    let meta_session = metadata_text(params, "session_id");
    let candidates = if request_correlation_in_logs {
        [own_trace, own_session, meta_trace, meta_session]
    } else {
        [own_session, own_trace, meta_session, meta_trace]
    };
    candidates
        .into_iter()
        .flatten()
        .next()
        .unwrap_or_else(|| fallback_trace_id.to_owned())
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
    params
        .litellm_session_id
        .clone()
        .or_else(|| metadata_text(params, "session_id"))
        .unwrap_or_else(|| fallback_session_id.to_owned())
}

fn metadata_text(params: &LitellmParams, key: &str) -> Option<String> {
    match params.metadata.as_ref()?.get(key)? {
        Json::Text(text) => Some(text.clone()),
        Json::Number(number) => Some(number.to_string()),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

mod tests;
