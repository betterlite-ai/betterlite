use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::Usage;
use indexmap::IndexMap;

pub mod error;
pub mod pricing;
pub mod realtime_stream;
pub mod responses_ws;
pub mod transcription;
mod usage;

mod tests;

pub type Event = IndexMap<String, Json>;

// Ports LiteLLMRealtimeStreamLoggingObject: the session's own records, their aggregate usage and the split tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeBilling {
    pub results: Vec<Event>,
    pub usage: Usage,
    pub service_tier: Option<String>,
}
