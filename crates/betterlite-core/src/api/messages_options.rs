pub mod error;
pub mod mock;
pub mod refusal;
mod tests;

use betterlite_anthropic::anthropic_capability_catalog::sampling::{
    SamplingParam, SamplingRequest, apply_sampling_param,
};
use betterlite_anthropic::anthropic_capability_catalog::{CatalogSources, DropParams};
use betterlite_anthropic::capabilities::speed::{
    SpeedDecision, SpeedParam, SpeedRequest, maybe_drop_speed_param,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use error::MessagesOptionsError;

pub const INCOMPLETE_STREAM_MESSAGE: &str = "Provider stream ended before emitting a message_stop event; the response is incomplete and any partial content (e.g. tool_use input JSON) may be truncated.";

// The keys of LiteLLM's AnthropicMessagesRequestOptionalParams TypedDict; every other key is dropped.
const OPTIONAL_PARAM_KEYS: [&str; 22] = [
    "max_tokens",
    "metadata",
    "stop_sequences",
    "stream",
    "system",
    "temperature",
    "thinking",
    "tool_choice",
    "tools",
    "top_k",
    "inference_geo",
    "top_p",
    "mcp_servers",
    "context_management",
    "compaction",
    "container",
    "output_format",
    "speed",
    "output_config",
    "cache_control",
    "reasoning_effort",
    "safeguards",
];

// LiteLLM runs the model gates only when the request resolved a model id.
pub struct ModelGate<'gate> {
    pub model: &'gate str,
    pub drop: DropParams,
    pub custom_llm_provider: Option<&'gate str>,
    pub catalogs: &'gate CatalogSources<'gate>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct SystemMessage {
    pub role: &'static str,
    pub content: Json,
}

pub fn requested_optional_params(
    params: &IndexMap<String, Json>,
    model_gate: Option<&ModelGate<'_>>,
) -> Result<IndexMap<String, Json>, MessagesOptionsError> {
    let mut filtered: IndexMap<String, Json> = params
        .iter()
        .filter(|(key, value)| OPTIONAL_PARAM_KEYS.contains(&key.as_str()) && **value != Json::Null)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let Some(gate) = model_gate else {
        return Ok(filtered);
    };
    let speed = SpeedRequest {
        model: gate.model,
        speed: filtered
            .get("speed")
            .map_or(SpeedParam::Absent, SpeedParam::Present),
        drop: gate.drop,
        custom_llm_provider: gate.custom_llm_provider,
    };
    if maybe_drop_speed_param(&speed, gate.catalogs)? == SpeedDecision::Drop {
        filtered.shift_remove("speed");
    }
    for param in [
        SamplingParam::Temperature,
        SamplingParam::TopP,
        SamplingParam::TopK,
    ] {
        let Some(value) = filtered.shift_remove(param.key()) else {
            continue;
        };
        let sampling = SamplingRequest {
            model: gate.model,
            param,
            value: &value,
            drop: gate.drop,
        };
        if let Some(kept) = apply_sampling_param(&sampling, gate.catalogs)? {
            filtered.insert(param.key().to_owned(), kept);
        }
    }
    Ok(filtered)
}

#[must_use]
pub fn system_message(system: &Json) -> Option<SystemMessage> {
    let present = match system {
        Json::Text(text) => !text.is_empty(),
        Json::List(blocks) => !blocks.is_empty(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => false,
    };
    present.then(|| SystemMessage {
        role: "system",
        content: system.clone(),
    })
}
