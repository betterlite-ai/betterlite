use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_translation::bedrock_streaming::BedrockGuardrailStreamParams;
use betterlite_core::routes::CallType;
use betterlite_prompts::response_tool_calls::structured::RouteCallTypes;

pub mod chunks;
pub mod errors;
pub mod frames;
pub mod input;
pub mod keepalive;
pub mod post_call;
pub mod response;
pub mod runtime;
pub mod streaming;
mod tests;

pub type RequestData = BTreeMap<String, GuardrailValue>;

// LiteLLM's _RESPONSES_API_CALL_TYPES: the frozenset of CallTypes::responses and CallTypes::aresponses.
pub const RESPONSES_API_CALL_TYPES: [CallType; 2] = [CallType::Responses, CallType::Aresponses];

pub struct StreamingSettings {
    pub guardrail_name: String,
    pub params: BedrockGuardrailStreamParams,
    pub mask_response_content: bool,
    pub keepalive_interval_seconds: Option<f64>,
}

// LiteLLM's _is_responses_api_route: only a known route whose get_call_types_for_route lists a Responses call type counts.
#[must_use]
pub fn is_responses_api_route<T>(request_route: Option<&str>, routes: &T) -> bool
where
    T: RouteCallTypes<CallType>,
{
    request_route
        .and_then(|route| routes.call_types_for_route(route))
        .is_some_and(|call_types| {
            call_types
                .iter()
                .any(|call_type| RESPONSES_API_CALL_TYPES.contains(call_type))
        })
}

// LiteLLM's _streams_incrementally: mask_response_content needs the full response, and the buffering flags decide the rest.
#[must_use]
pub const fn streams_incrementally(settings: &StreamingSettings) -> bool {
    if settings.mask_response_content {
        return false;
    }
    if !settings.params.buffer_until_moderated {
        return true;
    }
    settings.params.buffer_release_on_scan && !settings.params.streaming_end_of_stream_only
}
