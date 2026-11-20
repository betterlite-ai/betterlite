use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use indexmap::IndexMap;

use crate::api::responses_stream_errors::failure::FailureCause;

// One upstream Responses event: a dict-shaped event carries its mapping, and a provider event object the attributes getattr reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponsesEvent {
    pub attributes: ProviderEvent,
    pub mapping: Option<IndexMap<String, Json>>,
}

// A provider event's attributes as the JSON values getattr returns; a field the object lacks stays None.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderEvent {
    pub event_type: Option<Json>,
    pub item: Option<Json>,
    pub item_id: Option<Json>,
    pub delta: Option<Json>,
    pub summary_index: Option<Json>,
    pub response: Option<ProviderResponse>,
}

// The response object of a completed or failed provider event: output items keep their JSON form and usage its typed counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderResponse {
    pub output: Vec<Json>,
    pub status: Option<String>,
    pub usage: Option<ResponseApiUsage>,
    pub error: Option<ProviderError>,
}

// The error object of a failed provider response, whose message, type and code LiteLLM reads by attribute.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderError {
    pub message: Option<Json>,
    pub error_type: Option<Json>,
    pub code: Option<Json>,
}

// The upstream Responses iterator __anext__ pulls from: the next decoded event, its end, or the exception it raised.
#[async_trait]
pub trait ResponsesEventSource: Send {
    async fn next_event(&mut self) -> Result<Option<ResponsesEvent>, FailureCause>;
}
