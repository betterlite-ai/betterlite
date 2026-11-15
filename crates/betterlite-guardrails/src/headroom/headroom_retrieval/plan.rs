use std::collections::BTreeSet;

use betterlite_context::agentic_callback::{AgenticLogging, AgenticLoopCall};
use betterlite_context::agentic_loop_plan::{
    AgenticLoopPlan, AgenticLoopRequestPatch, JsonMap, get_text,
};
use serde_json::value::to_raw_value;

use crate::headroom::headroom_compression::HeadroomGuardrail;

use super::error::RetrievalError;
use super::followup::{self, Retrieved};
use super::response::ModelResponse;
use super::retrieve;
use super::tools::{self, RetrieveCall};

const CALL_ID_KEY: &str = "litellm_call_id";
const LOGGING_OBJECT_KEY: &str = "litellm_logging_obj";
const MAX_TOKENS_KEY: &str = "max_tokens";
const MODEL_KEY: &str = "model";
const HEADROOM_KEY_PREFIX: &str = "_headroom";
const TOOL_TYPE_KEY: &str = "tool_type";
const TOOL_TYPE: &str = "headroom_ccr";

pub(crate) async fn build(
    guardrail: &HeadroomGuardrail,
    call: &AgenticLoopCall<'_>,
) -> Result<AgenticLoopPlan, RetrievalError> {
    let response = ModelResponse::parse(call.response)?;
    let calls = match call.tools.get(tools::TOOL_CALLS_KEY) {
        Some(raw) => {
            serde_json::from_str(raw.get()).map_err(|source| RetrievalError::Tools { source })?
        }
        None => Vec::new(),
    };
    let call_id = resolve_call_id(call.logging, call.kwargs);
    let valid_hashes = guardrail.valid_hashes(call_id.as_deref());
    let retrieved = collect(guardrail, calls, &valid_hashes).await;
    let messages = followup::append(&response, call.messages, &retrieved)?;
    let mut metadata = JsonMap::new();
    metadata.insert(TOOL_TYPE_KEY.to_owned(), to_raw_value(&TOOL_TYPE)?);
    Ok(AgenticLoopPlan {
        run_agentic_loop: true,
        request_patch: Some(AgenticLoopRequestPatch {
            model: Some(full_model_name(call.logging, call.model)),
            messages: Some(messages),
            tools: None,
            max_tokens: max_tokens(call.optional_params, call.kwargs)?,
            optional_params: without_max_tokens(call.optional_params),
            kwargs: without_headroom_keys(call.kwargs),
        }),
        response_override: None,
        terminate: false,
        stop_reason: None,
        metadata,
    })
}

async fn collect(
    guardrail: &HeadroomGuardrail,
    calls: Vec<RetrieveCall>,
    valid_hashes: &BTreeSet<String>,
) -> Vec<Retrieved> {
    let mut retrieved = Vec::with_capacity(calls.len());
    for call in calls {
        let hash = tools::hash_argument(&call);
        let content = if valid_hashes.contains(&hash) {
            retrieve::fetch(guardrail, &hash, tools::query_argument(&call)).await
        } else {
            tracing::warn!(
                hash,
                "Headroom CCR: rejecting a hash this request did not issue"
            );
            format!("[Headroom: hash={hash} was not produced by the current request]")
        };
        tracing::debug!(
            hash,
            chars = content.chars().count(),
            "Headroom CCR: retrieved"
        );
        retrieved.push(Retrieved { call, content });
    }
    retrieved
}

fn resolve_call_id(logging: Option<&AgenticLogging>, kwargs: &JsonMap) -> Option<String> {
    if let Some(call_id) = logging
        .and_then(|record| record.call_id.as_deref())
        .filter(|call_id| !call_id.is_empty())
    {
        return Some(call_id.to_owned());
    }
    get_text(kwargs, CALL_ID_KEY)
}

fn full_model_name(logging: Option<&AgenticLogging>, fallback: &str) -> String {
    let candidate = logging
        .and_then(|record| record.agentic_loop_params.as_ref())
        .and_then(|params| get_text(params, MODEL_KEY));
    match candidate {
        Some(model) if !model.is_empty() => model,
        _ => fallback.to_owned(),
    }
}

fn max_tokens(optional: &JsonMap, kwargs: &JsonMap) -> Result<Option<u64>, RetrievalError> {
    if let Some(value) = read_max_tokens(optional)? {
        return Ok(Some(value));
    }
    read_max_tokens(kwargs)
}

fn read_max_tokens(map: &JsonMap) -> Result<Option<u64>, RetrievalError> {
    map.get(MAX_TOKENS_KEY).map_or(Ok(None), |raw| {
        serde_json::from_str::<Option<u64>>(raw.get())
            .map_err(|_error| RetrievalError::FieldType(MAX_TOKENS_KEY))
    })
}

fn without_max_tokens(params: &JsonMap) -> JsonMap {
    params
        .iter()
        .filter(|(key, _)| key.as_str() != MAX_TOKENS_KEY)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn without_headroom_keys(kwargs: &JsonMap) -> JsonMap {
    kwargs
        .iter()
        .filter(|(key, _)| {
            !key.starts_with(HEADROOM_KEY_PREFIX) && key.as_str() != LOGGING_OBJECT_KEY
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
