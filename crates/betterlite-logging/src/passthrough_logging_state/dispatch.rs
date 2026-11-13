use async_trait::async_trait;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use super::response::PassThroughResponse;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_state::LoggingState;
use crate::logging_state::correlation::CorrelationContext;
use crate::logging_success::dispatch::dispatch_success_handlers;
use crate::logging_success::handler::SuccessHandler;
use crate::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use crate::logging_success::types::{SuccessRequest, SuccessResult};

// LiteLLM's pass-through success kwargs, which its handlers fill and the log path reads back
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PassThroughLoggingKwargs {
    pub entries: IndexMap<String, Json>,
}

pub struct PassThroughLogCall<'call> {
    pub handler: &'call SuccessHandler<'call>,
    pub state: &'call mut LoggingState,
    pub correlation: &'call mut CorrelationContext,
    pub response: Option<PassThroughResponse>,
    pub result: &'call str,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub cache_hit: bool,
    pub kwargs: PassThroughLoggingKwargs,
}

#[async_trait]
pub trait PassThroughLogDispatch: Send + Sync {
    async fn dispatch(&self, call: PassThroughLogCall<'_>);
}

// LiteLLM's _handle_logging: the caller's result string and cache_hit never reach the dispatch
pub async fn handle_logging(call: PassThroughLogCall<'_>) {
    let PassThroughLogCall {
        handler,
        state,
        correlation,
        response,
        start_time,
        end_time,
        kwargs,
        ..
    } = call;
    let result = response.map_or_else(|| SuccessResult::Other(Json::Null), SuccessResult::from);
    let request = SuccessRequest {
        result,
        start_time: Some(start_time),
        end_time: Some(end_time),
        cache_hit: Some(false),
        prefer_async_handlers: true,
        standard_logging_object: standard_logging_object_kwarg(&kwargs),
        guardrail_snapshot: None,
    };
    dispatch_success_handlers(handler, state, correlation, &request).await;
}

fn standard_logging_object_kwarg(
    kwargs: &PassThroughLoggingKwargs,
) -> Option<StandardLoggingPayload> {
    let value = kwargs.entries.get(STANDARD_LOGGING_OBJECT)?;
    let text = serde_json::to_string(value).ok()?;
    match serde_json::from_str(&text) {
        Ok(payload) => Some(payload),
        Err(error) => {
            tracing::warn!(%error, "the pass-through standard logging object could not be read");
            None
        }
    }
}
