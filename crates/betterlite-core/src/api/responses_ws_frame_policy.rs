use std::borrow::Cow;
use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::WsRequestDefaults;
use indexmap::IndexMap;
use serde_json::value::RawValue;
use thiserror::Error;

use crate::api::responses_stream_events::ProjectQuotaCallback;

pub mod events;
pub mod quota;
pub mod request;
pub mod request_masking;
pub mod response;

mod tests;

// a response.create frame that never declares an output cap is still charged against an OTPM quota
pub const NO_MAX_OUTPUT_TOKENS_FLOOR: i64 = 1024;
pub const CHARS_PER_TOKEN_ESTIMATE: usize = 4;

pub const LOGGED_EVENT_TYPES: [&str; 5] = [
    "response.created",
    "response.completed",
    "response.failed",
    "response.incomplete",
    "error",
];
pub const MASKABLE_TEXT_BLOCK_TYPES: [&str; 3] = ["input_text", "output_text", "text"];
pub const FAILURE_EVENT_TYPES: [&str; 2] = ["error", "response.failed"];
pub const OUTPUT_ITEM_EVENT_TYPES: [&str; 2] =
    ["response.output_item.added", "response.output_item.done"];
pub const DELTA_EVENT_TYPES: [&str; 4] = [
    "response.output_text.delta",
    "response.reasoning_summary_text.delta",
    "response.refusal.delta",
    "response.function_call_arguments.delta",
];
// the relay drops these while output masking runs, since response.completed carries the masked copy
pub const OUTPUT_DONE_EVENT_TYPES: [&str; 6] = [
    "response.output_text.done",
    "response.content_part.done",
    "response.output_item.done",
    "response.function_call_arguments.done",
    "response.reasoning_summary_text.done",
    "response.reasoning_summary_part.done",
];

pub type UnmaskPiiText = dyn Fn(&str, &IndexMap<String, String>) -> String + Send + Sync;
pub type RequestData = BTreeMap<String, Box<RawValue>>;

#[derive(Debug, Error)]
pub enum FramePolicyError<GuardError>
where
    GuardError: Error + 'static,
{
    // LiteLLM's .get("type") raises AttributeError here, and the relay ends the connection on it
    #[error("the frame is JSON but not an object, so it can't be relayed")]
    NotAnObject,
    // json.loads reads a lone surrogate escape or NaN that serde_json refuses, so such an object fails closed
    #[error("the frame opens a JSON object that can't be read: {source}")]
    Unreadable {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Guardrail(GuardError),
    #[error("the frame can't be written back as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

pub struct InputGuardrail<'policy, Guard> {
    pub callback: Guard,
    // B3805 hands its unmask function over beside the callback, which has no unmask method of its own
    pub unmask_pii_text: &'policy UnmaskPiiText,
}

pub struct FramePolicy<'policy, Guard> {
    pub quota_callbacks: &'policy [Arc<dyn ProjectQuotaCallback>],
    pub user_api_key_dict: Option<&'policy UserAPIKeyAuth>,
    pub guardrail_callbacks: &'policy [InputGuardrail<'policy, Guard>],
    pub output_guardrail_callbacks: &'policy [Guard],
    pub authorized_model: Option<&'policy str>,
    pub request_defaults: Option<&'policy WsRequestDefaults<Json>>,
}

pub(crate) struct PiiCall<'call, Guard, Settings> {
    pub(crate) guard: &'call Guard,
    pub(crate) settings: Option<&'call Settings>,
    pub(crate) request_data: &'call RequestData,
    pub(crate) output_parse_pii: bool,
}

// json.loads, then .get on whatever came back: text that isn't JSON passes through untouched, unless it
// opens an object, since forwarding that unread would skip model enforcement and masking
pub(crate) fn decode_object<GuardError>(
    message: &str,
) -> Result<Option<IndexMap<String, Json>>, FramePolicyError<GuardError>>
where
    GuardError: Error + 'static,
{
    match serde_json::from_str::<Json>(message) {
        Ok(Json::Object(fields)) => Ok(Some(fields)),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(FramePolicyError::NotAnObject)
        }
        Err(source) if message.trim_start().starts_with('{') => {
            Err(FramePolicyError::Unreadable { source })
        }
        Err(_invalid) => Ok(None),
    }
}

pub(crate) fn encode<GuardError>(
    fields: IndexMap<String, Json>,
) -> Result<Cow<'static, str>, FramePolicyError<GuardError>>
where
    GuardError: Error + 'static,
{
    serde_json::to_string(&Json::Object(fields))
        .map(Cow::Owned)
        .map_err(|source| FramePolicyError::Encode { source })
}
