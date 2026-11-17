use betterlite_core::api::responses_stream_primitives::state::{
    BaseStreamStateInput, LoggingSnapshot, ResponsesStreamBaseState, SharedResponsesApiBase,
    StreamTimes, base_stream_state,
};
use betterlite_models::api_base_lookup::ApiBaseContext;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use chrono::{DateTime, TimeDelta};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use super::support::{NoHooks, SharedResolution, failure, resolution_context};

// the fields the Python probe receives, and the logging state the base constructor reads
#[derive(Debug, Deserialize)]
pub(crate) struct ProbeSpec {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) provider: Option<String>,
    #[serde(default)]
    pub(crate) litellm_metadata: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) request_data: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) call_type: Option<String>,
    pub(crate) start_time: String,
    pub(crate) created: f64,
    #[serde(default)]
    pub(crate) litellm_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) headers: IndexMap<String, String>,
}

pub(crate) fn answer(rows: &[ProbeSpec]) -> Result<Value, Box<HelperError>> {
    let mut answers = Vec::with_capacity(rows.len());
    for row in rows {
        answers.push(state_answer(row)?);
    }
    Ok(json!({ "rows": answers }))
}

pub(crate) fn build_state(
    spec: &ProbeSpec,
    max_streaming_duration_seconds: Option<f64>,
) -> Result<ResponsesStreamBaseState, Box<HelperError>> {
    let shared = SharedResolution::default();
    let resolution = resolution_context(&shared);
    let resolver = SharedResponsesApiBase {
        context: ApiBaseContext {
            model_alias_map: &shared.aliases,
            hooks: &NoHooks,
            resolution: &resolution,
        },
    };
    let start_time = DateTime::parse_from_rfc3339(&spec.start_time)
        .map_err(|error| failure(error.to_string()))?;
    Ok(base_stream_state(&BaseStreamStateInput {
        model: &spec.model,
        custom_llm_provider: spec.provider.as_deref(),
        litellm_metadata: spec.litellm_metadata.as_ref(),
        request_data: spec.request_data.as_ref(),
        call_type: spec.call_type.as_deref(),
        logging: LoggingSnapshot {
            start_time: Some(start_time),
            litellm_params: spec.litellm_params.clone(),
        },
        headers: &spec.headers,
        api_base: &resolver,
        times: StreamTimes {
            // the fallback clock is a different instant, so the case fails when the logged start time is ignored
            now: start_time + TimeDelta::hours(12),
            created_timestamp: spec.created,
        },
        max_streaming_duration_seconds,
    }))
}

fn state_answer(spec: &ProbeSpec) -> Result<Value, Box<HelperError>> {
    let state = build_state(spec, None)?;
    let hidden = &state.hidden_params;
    let start_time = state.start_time.to_rfc3339();
    Ok(json!({
        "model": state.model,
        "custom_llm_provider": state.custom_llm_provider,
        "litellm_metadata": state.litellm_metadata,
        "request_data": state.request_data,
        "call_type": state.call_type,
        "start_time": start_time,
        "created_timestamp": state.created_timestamp,
        "finished": state.finished,
        "failure_handled": state.failure_handled,
        "yielded_first_chunk": state.yielded_first_chunk,
        "output_started": state.output_started,
        "completed_response_cache_hit": state.completed_response_cache_hit,
        "completed_response_cached": state.completed_response_cached,
        "completed_response_logged": state.completed_response_logged,
        "persist_before_logging": state.persist_before_logging,
        "generated_content": state.generated_content,
        "generated_tool_arguments": state.generated_tool_arguments,
        "pending_tasks": state.pending_tasks.len(),
        "hidden_params": {
            "model_id": hidden.model_id,
            "api_base": hidden.api_base,
            "custom_llm_provider": hidden.custom_llm_provider,
            "additional_headers": hidden.additional_headers,
        },
        "raw_response_headers": state.raw_response_headers,
    }))
}
