use std::time::Duration;

use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::handler::kwargs::request_stream;
use betterlite_cache::replay::ReplayRequest;
use betterlite_cache::types::call_type::CachingSupportedCallType;
use chrono::{DateTime, FixedOffset};

use super::error::LookupError;
use super::json::request_text;

// What _async_get_cache and _sync_get_cache receive besides the logging object
pub struct CacheLookup {
    pub model: String,
    pub call_type: CachingSupportedCallType,
    pub start_time: DateTime<FixedOffset>,
    pub kwargs: KeyObject,
    pub args: Vec<KeyValue>,
    // the agentic loop's own setting, which decides whether a converted stream replays from the cache
    pub converted_stream_requested: bool,
    pub agentic_loop_depth: u32,
}

pub fn replay_request<'request>(
    lookup: &'request CacheLookup,
    kwargs: &'request KeyObject,
    model: &'request str,
    end_time: DateTime<FixedOffset>,
) -> Result<ReplayRequest<'request>, LookupError> {
    let elapsed = end_time
        .signed_duration_since(lookup.start_time)
        .to_std()
        .unwrap_or(Duration::ZERO);
    Ok(ReplayRequest {
        stream: request_stream(kwargs)?,
        converted_stream_requested: lookup.converted_stream_requested,
        agentic_loop_depth: lookup.agentic_loop_depth,
        model,
        custom_llm_provider: request_text(kwargs, "custom_llm_provider")?,
        elapsed,
    })
}
