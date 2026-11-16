use std::collections::BTreeSet;
use std::convert::Infallible;
use std::error::Error;
use std::str::Utf8Error;

use betterlite_context::agentic_callback::AgenticLogging;
use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};
use betterlite_models::decisions::Json;
use thiserror::Error;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, JsonBoundary};
use crate::api::messages_sse_events::stream::MessagesProviderStream;

pub mod events;
pub mod interceptor;
pub mod stream;
mod tests;

// LiteLLM's HOLD_BACK_PING_INTERVAL_SECONDS
pub const PING_INTERVAL_SECONDS: f64 = 15.0;

// LiteLLM's STREAM_SSE_KEEPALIVE_PING_BYTES, sent whenever no other byte is ready
pub const KEEPALIVE_PING: &[u8] = b"event: ping\ndata: {\"type\": \"ping\"}\n\n";

// LiteLLM's SERVER_FULFILLED_TOOL_LEAK_ERROR_SSE_BYTES, which ends the turn instead of leaking the call
pub const SERVER_FULFILLED_TOOL_LEAK_ERROR: &[u8] = b"event: error\ndata: {\"type\": \"error\", \"error\": {\"type\": \"api_error\", \"message\": \"Server-side tool retrieval failed, so this turn could not be completed. Please retry.\"}}\n\n";

// what the iterator is constructed with; the interceptor reads it when a hook fires
#[derive(Clone, Debug)]
pub struct MessagesAgenticStreamInput {
    pub model: String,
    pub messages: Vec<FollowupMessage>,
    pub optional_request_params: JsonMap,
    pub logging: AgenticLogging,
    pub custom_llm_provider: String,
    pub kwargs: JsonMap,
    // None is LiteLLM's hold_back False; Some keeps the provider bytes behind keepalive pings
    pub hold_back: Option<HoldBackPolicy>,
}

// LiteLLM's hold_back, server_fulfilled_tool_names and ping_interval_seconds together
#[derive(Clone, Debug, PartialEq)]
pub struct HoldBackPolicy {
    pub server_fulfilled_tool_names: BTreeSet<String>,
    pub ping_interval_seconds: f64,
}

impl Default for HoldBackPolicy {
    fn default() -> Self {
        Self {
            server_fulfilled_tool_names: BTreeSet::new(),
            ping_interval_seconds: PING_INTERVAL_SECONDS,
        }
    }
}

// what the agentic completion hooks answered, one variant per result LiteLLM distinguishes
#[derive(Debug)]
pub enum MessagesAgenticStreamResult<FollowUp> {
    NoFollowUp,
    Stream(FollowUp),
    Message(Json),
    // a hook value no iterator can serve, named as Python names it for the warning
    Unexpected { type_name: String },
}

#[derive(Debug, Error)]
pub enum MessagesAgenticError<Provider = Infallible, FollowUp = Infallible>
where
    Provider: Error + 'static,
    FollowUp: Error + 'static,
{
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("the agentic stream payload is not valid JSON")]
    Json(#[source] serde_json::Error),
    #[error("the agentic stream payload is not valid UTF-8")]
    Utf8(#[source] Utf8Error),
    // LiteLLM's duck-typed rebuild raises here; the hook processing then logs and skips the hooks
    #[error("the {field} field of an agentic stream event is not {expected}")]
    Shape {
        field: String,
        expected: &'static str,
    },
    #[error(transparent)]
    Provider(Provider),
    #[error(transparent)]
    FollowUp(FollowUp),
}

pub type MessagesAgenticStreamError<S, I> = MessagesAgenticError<
    <S as MessagesProviderStream>::Error,
    <<I as interceptor::AgenticInterceptor>::FollowUp as MessagesProviderStream>::Error,
>;

// the boundary read for json.loads input data_str (L72) and json.loads input partial (L146)
pub fn decode_messages_agentic_stream(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<Json, MessagesAgenticError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    serde_json::from_slice(bytes).map_err(MessagesAgenticError::Json)
}

#[must_use]
// ast-grep-ignore: no-long-names, no-one-line-functions
pub fn is_server_fulfilled_tool_leak_error(chunk: &[u8]) -> bool {
    chunk == SERVER_FULFILLED_TOOL_LEAK_ERROR
}
