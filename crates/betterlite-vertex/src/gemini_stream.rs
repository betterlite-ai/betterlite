pub mod adapters;
mod buffer;
pub mod chunk;
mod embedded;
pub mod error;
mod parser;
mod tests;
use crate::gemini_response::complete::SERVICE_TIER_HEADER;
use crate::gemini_response_parts::projection::CallMode;
use buffer::FragmentBuffer;
use http::header::HeaderMap;
use uuid::Uuid;
#[derive(Debug, Clone, Copy)]
pub struct StreamOptions {
    pub now: i64,
    pub call_mode: CallMode,
}
pub struct GeminiStreamDecoder {
    options: StreamOptions,
    ids: Box<dyn FnMut() -> Uuid + Send>,
    service_tier: Option<String>,
    accumulated: bool,
    buffer: FragmentBuffer,
    cumulative_tool_call_index: u32,
    has_seen_tool_calls: bool,
}
fn stream_service_tier(headers: &HeaderMap) -> Option<String> {
    let mut tier = String::new();
    for value in headers.get_all(SERVICE_TIER_HEADER) {
        if !tier.is_empty() {
            tier.push_str(", ");
        }
        tier.extend(value.as_bytes().iter().map(|byte| char::from(*byte)));
    }
    match tier.to_lowercase().as_str() {
        "" => None,
        "standard" => Some("default".to_owned()),
        other => Some(other.to_owned()),
    }
}
