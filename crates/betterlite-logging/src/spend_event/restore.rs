use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use rust_decimal::serde::arbitrary_precision_option;
use serde::Serialize;

use crate::spend_event::event::{DynamicParams, LitellmParams, Mapping, SpendEvent};
use crate::spend_event::response::CompactResponse;
use crate::spend_event::time::EventTime;
use crate::spend_event::tools::{FunctionTool, function_tools};
use crate::spend_event::usage::{RestoredUsage, UsageError, restore_usage};

// The kwargs carry the key only when the original call had one, even when its value was null
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum StreamingResponse {
    Absent,
    Restored(Option<Box<CompactResponse>>),
}

impl StreamingResponse {
    #[must_use]
    pub const fn is_absent(&self) -> bool {
        match self {
            Self::Absent => true,
            Self::Restored(_) => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CallbackKwargs {
    pub litellm_call_id: Option<String>,
    pub call_type: Option<String>,
    pub model: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub stream: Option<bool>,
    pub cache_hit: Option<bool>,
    #[serde(with = "arbitrary_precision_option")]
    pub response_cost: Option<Decimal>,
    pub completion_start_time: Option<EventTime>,
    pub agent_id: Option<String>,
    pub litellm_trace_id: Option<String>,
    pub litellm_params: LitellmParams,
    pub standard_logging_object: Option<Mapping>,
    #[serde(rename = "standard_callback_dynamic_params")]
    pub dynamic_params: Option<DynamicParams>,
    pub combined_usage_object: Option<RestoredUsage>,
    pub realtime_tools: Option<Vec<Json>>,
    pub realtime_tool_calls: Option<Vec<Json>>,
    pub tools: Option<Vec<FunctionTool>>,
    #[serde(skip_serializing_if = "StreamingResponse::is_absent")]
    pub complete_streaming_response: StreamingResponse,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CallbackArgs {
    pub kwargs: CallbackKwargs,
    pub response_obj: Option<CompactResponse>,
    pub start_time: EventTime,
    pub end_time: EventTime,
}

// Rebuilds what the in-process cost callback receives, so the collector runs the unchanged pipeline
pub fn callback_args(event: SpendEvent) -> Result<CallbackArgs, UsageError> {
    let combined_usage_object = event.combined_usage.map(restore_usage).transpose()?;
    let tools = function_tools(event.request_tool_names);
    let complete_streaming_response = if event.complete_streaming_response {
        StreamingResponse::Restored(event.response.clone().map(Box::new))
    } else {
        StreamingResponse::Absent
    };
    Ok(CallbackArgs {
        kwargs: CallbackKwargs {
            litellm_call_id: event.litellm_call_id,
            call_type: event.call_type,
            model: event.model,
            custom_llm_provider: event.custom_llm_provider,
            stream: event.stream,
            cache_hit: event.cache_hit,
            response_cost: event.response_cost,
            completion_start_time: event.completion_start_time,
            agent_id: event.agent_id,
            litellm_trace_id: event.litellm_trace_id,
            litellm_params: event.litellm_params,
            standard_logging_object: event.standard_logging_object,
            dynamic_params: event.dynamic_params,
            combined_usage_object,
            realtime_tools: event.realtime_tools,
            realtime_tool_calls: event.realtime_tool_calls,
            tools: (!tools.is_empty()).then_some(tools),
            complete_streaming_response,
        },
        response_obj: event.response,
        start_time: event.start_time,
        end_time: event.end_time,
    })
}
