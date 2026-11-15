use rust_decimal::Decimal;
use rust_decimal::serde::arbitrary_precision_option;
use serde::Deserialize;

use crate::spend_event::event::{DynamicParams, LitellmParams, Mapping};
use crate::spend_event::presence::Presence;
use crate::spend_event::time::EventTime;
use betterlite_models::decisions::Json;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachePresence {
    Off,
    On,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResponseBody {
    pub fields: Mapping,
    pub tool_call_names: Vec<String>,
}

// The caller says which kind of object the response was, because a mapping and a typed model can carry the same keys
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseObject {
    Absent,
    Model(ResponseBody),
    Mapping(ResponseBody),
    Batch(ResponseBody),
    Interactions(ResponseBody),
}

impl ResponseObject {
    // Batch retrieves and interaction polls branch on the concrete class, so they never leave the process
    #[must_use]
    pub const fn is_offloadable(&self) -> bool {
        match self {
            Self::Batch(_) | Self::Interactions(_) => false,
            Self::Absent | Self::Model(_) | Self::Mapping(_) => true,
        }
    }

    #[must_use]
    pub const fn body(&self) -> Option<&ResponseBody> {
        match self {
            Self::Absent => None,
            Self::Model(body)
            | Self::Mapping(body)
            | Self::Batch(body)
            | Self::Interactions(body) => Some(body),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedAuth {
    pub budget_reservation: Option<Mapping>,
}

// A typed auth object in a bucket is told apart by the caller, never guessed from the bucket's keys
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypedAuths {
    pub metadata: Option<TypedAuth>,
    pub litellm_metadata: Option<TypedAuth>,
}

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub tools: Option<Vec<Mapping>>,
}

#[derive(Debug, Deserialize)]
pub struct PassthroughPayload {
    pub request_body: Option<RequestBody>,
}

#[derive(Debug, Deserialize)]
pub struct SuccessKwargs {
    pub litellm_call_id: Option<String>,
    pub call_type: Option<String>,
    pub model: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub stream: Option<bool>,
    #[serde(default)]
    pub complete_streaming_response: Presence,
    pub cache_hit: Option<bool>,
    #[serde(default, with = "arbitrary_precision_option")]
    pub response_cost: Option<Decimal>,
    pub completion_start_time: Option<EventTime>,
    pub agent_id: Option<String>,
    pub litellm_trace_id: Option<String>,
    #[serde(default)]
    pub litellm_params: LitellmParams,
    pub standard_logging_object: Option<Mapping>,
    #[serde(rename = "standard_callback_dynamic_params")]
    pub dynamic_params: Option<DynamicParams>,
    pub combined_usage_object: Option<Mapping>,
    pub realtime_tools: Option<Vec<Json>>,
    pub realtime_tool_calls: Option<Vec<Json>>,
    pub tools: Option<Vec<Mapping>>,
    pub passthrough_logging_payload: Option<PassthroughPayload>,
}

pub struct BuildInput<'data> {
    pub kwargs: &'data Mapping,
    pub typed_auth: &'data TypedAuths,
    pub response: &'data ResponseObject,
    pub start_time: EventTime,
    pub end_time: EventTime,
    pub store_bodies: bool,
    pub cache: CachePresence,
}
