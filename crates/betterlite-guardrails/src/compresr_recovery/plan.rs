use std::time::Duration;

use betterlite_context::agentic_callback::{AgenticCall, AgenticGate, AgenticLoopCall};
use betterlite_context::agentic_loop_plan::{
    AgenticLoopPlan, AgenticLoopRequestPatch, FollowupMessage, JsonMap, get_text,
};
use serde_json::value::to_raw_value;

use super::error::RecoveryError;
use super::followup;
use super::kwargs::follow_up_kwargs;
use super::response::ModelResponse;
use super::retrieval::resolve_retrievals;
use super::scope::caller_scope;
use super::store::{RecoveryStore, StoreKey};
use super::tools::{
    COMPRESR_RETRIEVE_TOOL_NAME, RetrieveCall, TOOL_CALLS_KEY, holds_retrieve_tool, retrieve_calls,
};

const MAX_TOKENS_KEY: &str = "max_tokens";
const MODEL_KEY: &str = "model";
const TOOL_TYPE_KEY: &str = "tool_type";

pub struct PlanRequest<'call> {
    pub call: AgenticLoopCall<'call>,
    pub own_marker: Option<&'call str>,
    pub now: Duration,
}

// The loop runs only when the request carries the retrieve tool and the model called it
pub fn should_run(call: &AgenticCall<'_>) -> Result<AgenticGate, RecoveryError> {
    if !holds_retrieve_tool(call.tools.unwrap_or_default()) {
        return Ok(AgenticGate::default());
    }
    let response = ModelResponse::parse(call.response)?;
    let calls = retrieve_calls(&response.calls);
    if calls.is_empty() {
        return Ok(AgenticGate::default());
    }
    let mut tools = JsonMap::new();
    tools.insert(TOOL_CALLS_KEY.to_owned(), to_raw_value(&calls)?);
    Ok(AgenticGate { run: true, tools })
}

// A plan that retrieved nothing skips the extra provider round trip
pub fn build_plan(
    store: &RecoveryStore,
    request: &PlanRequest<'_>,
) -> Result<AgenticLoopPlan, RecoveryError> {
    let call = &request.call;
    let calls: Vec<RetrieveCall> = match call.tools.get(TOOL_CALLS_KEY) {
        Some(raw) => {
            serde_json::from_str(raw.get()).map_err(|source| RecoveryError::ToolCalls { source })?
        }
        None => Vec::new(),
    };
    let pruned = store.prune(request.now);
    tracing::debug!(pruned, "Compresr: pruned the recovery store");
    // LiteLLM reads the logging object's call id only, never the request's litellm_call_id
    let call_id = call
        .logging
        .and_then(|record| record.call_id.as_deref())
        .filter(|call_id| !call_id.is_empty());
    let key = StoreKey::resolve(call_id, &caller_scope(call.kwargs));
    let resolution = resolve_retrievals(store, key.as_ref(), calls, request.now);
    if !resolution.resolved_any {
        return Ok(AgenticLoopPlan::default());
    }
    let response = ModelResponse::parse(call.response)?;
    let messages = followup::append(&response, call.messages, &resolution.retrieved)?;
    let mut metadata = JsonMap::new();
    metadata.insert(
        TOOL_TYPE_KEY.to_owned(),
        to_raw_value(COMPRESR_RETRIEVE_TOOL_NAME)?,
    );
    Ok(AgenticLoopPlan {
        run_agentic_loop: true,
        request_patch: Some(patch(request, messages)?),
        response_override: None,
        terminate: false,
        stop_reason: None,
        metadata,
    })
}

fn patch(
    request: &PlanRequest<'_>,
    messages: Vec<FollowupMessage>,
) -> Result<AgenticLoopRequestPatch, RecoveryError> {
    let call = &request.call;
    let model = call
        .logging
        .and_then(|record| record.agentic_loop_params.as_ref())
        .and_then(|params| get_text(params, MODEL_KEY))
        .filter(|candidate| !candidate.is_empty())
        .unwrap_or_else(|| call.model.to_owned());
    let optional_params = call
        .optional_params
        .iter()
        .filter(|(key, _)| key.as_str() != MAX_TOKENS_KEY)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let from_optional = read_max_tokens(call.optional_params)?;
    let max_tokens = if from_optional.is_some() {
        from_optional
    } else {
        read_max_tokens(call.kwargs)?
    };
    Ok(AgenticLoopRequestPatch {
        model: Some(model),
        messages: Some(messages),
        tools: None,
        max_tokens,
        optional_params,
        kwargs: follow_up_kwargs(call.kwargs, request.own_marker)?,
    })
}

fn read_max_tokens(params: &JsonMap) -> Result<Option<u64>, RecoveryError> {
    let Some(raw) = params.get(MAX_TOKENS_KEY) else {
        return Ok(None);
    };
    serde_json::from_str(raw.get()).map_err(|source| RecoveryError::FieldType {
        field: MAX_TOKENS_KEY,
        source,
    })
}
