use std::collections::{BTreeMap, BTreeSet};

use betterlite_context::agentic_callback::AgenticCallback;
use betterlite_context::agentic_followup_kwargs::{
    AgenticFollowupInput, AgenticKwargValue, build_agentic_followup_kwargs,
};
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, AgenticLoopRequestPatch, JsonMap};
use betterlite_models::decisions::Json;
use betterlite_openai::code_execution_records::{
    INTERCEPTION_INTERNAL_PREFIXES, OTHER_INTERCEPTION_PREFIXES,
};
use indexmap::IndexMap;
use serde_json::Number;
use serde_json::from_str;
use serde_json::value::{RawValue, to_raw_value};

use super::error::ChatCallError;
use super::fake_stream::FakeStreamPort;
use super::settings::{AgenticSettings, converted_stream_requested};
use super::{
    AGENTIC_LOOP_API_SURFACE_KEY, AGENTIC_LOOP_DEPTH_KEY, AGENTIC_LOOP_FINGERPRINTS_KEY,
    AGENTIC_LOOP_PREFIX, CUSTOM_LLM_PROVIDER_KEY, LITELLM_METADATA_KEY, MAX_AGENTIC_LOOPS_KEY,
    MESSAGES_KEY, MODEL_KEY, TOOL_CHOICE_KEY, TOOLS_KEY,
};
use super::{ChatContinuation, ChatLoopResult, LoggingHandle};

mod tests;

const FOLLOWUP_INTERNAL_PARAMS: [&str; 7] = [
    "acompletion",
    "litellm_logging_obj",
    CUSTOM_LLM_PROVIDER_KEY,
    "model_alias_map",
    "stream_response",
    "custom_prompt_dict",
    AGENTIC_LOOP_API_SURFACE_KEY,
];

const INJECTED_KEYS: [&str; 3] = [
    AGENTIC_LOOP_DEPTH_KEY,
    AGENTIC_LOOP_FINGERPRINTS_KEY,
    MAX_AGENTIC_LOOPS_KEY,
];

pub(crate) struct FollowupCall<'call, Continuation: ?Sized, FakeStream: ?Sized> {
    pub plan: &'call AgenticLoopPlan,
    pub callback: &'call dyn AgenticCallback,
    pub model: &'call str,
    pub optional_params: &'call JsonMap,
    pub kwargs: &'call JsonMap,
    pub logging: LoggingHandle<'call>,
    pub custom_llm_provider: &'call str,
    pub fingerprint: &'call str,
    pub settings: &'call AgenticSettings,
    pub continuation: &'call Continuation,
    pub fake_stream: &'call FakeStream,
}

pub(crate) async fn execute<Continuation, FakeStream>(
    call: &FollowupCall<'_, Continuation, FakeStream>,
) -> Result<ChatLoopResult, ChatCallError<Continuation::Error>>
where
    Continuation: ChatContinuation + ?Sized,
    FakeStream: FakeStreamPort + ?Sized,
{
    let empty = AgenticLoopRequestPatch::default();
    let patch = call.plan.request_patch.as_ref().unwrap_or(&empty);
    let Some(messages) = patch.messages.as_deref() else {
        return Err(ChatCallError::MissingMessages);
    };
    let model = followup_model(patch, call.model, call.custom_llm_provider);
    let params = followup_params(call.optional_params, patch)?;
    let kwargs = followup_kwargs(call, &params)?;
    let outcome = match call
        .continuation
        .complete(&model, messages, &params, &kwargs)
        .await
    {
        Ok(response) => Ok(post_process(response, call).await),
        Err(source) => Err(ChatCallError::Completion(source)),
    };
    if let Err(error) = call
        .callback
        .agentic_loop_cleanup(call.plan, call.kwargs)
        .await
    {
        let call_id = call.logging.call_id();
        let log_model = call.model;
        tracing::error!(
            "LiteLLM.AgenticHookError: Exception in async_agentic_loop_cleanup_hook [call_id={call_id} model={log_model}]: {error}"
        );
    }
    outcome
}

async fn post_process<Continuation, FakeStream>(
    response: ChatLoopResult,
    call: &FollowupCall<'_, Continuation, FakeStream>,
) -> ChatLoopResult
where
    Continuation: ?Sized,
    FakeStream: FakeStreamPort + ?Sized,
{
    let hooked = match response {
        ChatLoopResult::Response(response) => {
            let hooked = if call.callback.overrides().post_response {
                let original = response.clone();
                match call
                    .callback
                    .post_agentic_loop_response(response, call.plan, call.kwargs)
                    .await
                {
                    Ok(hooked) => hooked,
                    Err(error) => {
                        let call_id = call.logging.call_id();
                        let log_model = call.model;
                        tracing::error!(
                            "LiteLLM.AgenticHookError: Exception in async_post_agentic_loop_response_hook [call_id={call_id} model={log_model}]: {error}"
                        );
                        original
                    }
                }
            } else {
                response
            };
            ChatLoopResult::Response(hooked)
        }
        stream @ ChatLoopResult::Stream(_) => stream,
    };
    if converted_stream_requested(call.kwargs) && call.settings.depth == 0 {
        return call
            .fake_stream
            .wrap(hooked, call.model, call.custom_llm_provider, &call.logging);
    }
    hooked
}

fn followup_model(
    patch: &AgenticLoopRequestPatch,
    model: &str,
    custom_llm_provider: &str,
) -> String {
    let chosen = patch.model.as_deref().unwrap_or(model);
    if chosen.contains('/') {
        return chosen.to_owned();
    }
    format!("{custom_llm_provider}/{chosen}")
}

fn followup_params(
    optional_params: &JsonMap,
    patch: &AgenticLoopRequestPatch,
) -> Result<JsonMap, serde_json::Error> {
    let mut params = optional_params.clone();
    for (key, value) in &patch.optional_params {
        params.insert(key.clone(), value.clone());
    }
    if let Some(tools) = &patch.tools {
        params.insert(TOOLS_KEY.to_owned(), to_raw_value(tools)?);
    }
    if !patch.optional_params.contains_key(TOOL_CHOICE_KEY) {
        params.shift_remove(TOOL_CHOICE_KEY);
    }
    Ok(params)
}

fn followup_kwargs<Continuation, FakeStream>(
    call: &FollowupCall<'_, Continuation, FakeStream>,
    params: &JsonMap,
) -> Result<JsonMap, ChatCallError<Continuation::Error>>
where
    Continuation: ChatContinuation + ?Sized,
    FakeStream: ?Sized,
{
    let request_params = request_param_names(params);
    let request_kwargs = filtered_kwargs(call.kwargs);
    let patch = call.plan.request_patch.as_ref();
    let patch_kwargs = patch.map_or_else(JsonMap::new, |patched| filtered_kwargs(&patched.kwargs));
    let request_values = key_values(&request_kwargs);
    let patch_values = key_values(&patch_kwargs);
    let projection = build_agentic_followup_kwargs(&AgenticFollowupInput {
        request_kwargs: &request_values,
        patch_kwargs: &patch_values,
        request_params: &request_params,
        depth: call.settings.depth,
        max_loops: call.settings.max_loops,
        fingerprints: &call.settings.fingerprints,
        fingerprint: call.fingerprint,
    })?;
    let mut kwargs = JsonMap::new();
    for (key, value) in request_kwargs.iter().chain(&patch_kwargs) {
        if projection.as_ref().contains_key(key) {
            kwargs.insert(key.clone(), value.clone());
        }
    }
    for (key, value) in projection.as_ref() {
        if INJECTED_KEYS.contains(&key.as_str()) {
            kwargs.insert(key.clone(), to_raw_value(&kwarg_json(value))?);
        }
    }
    Ok(with_metadata(kwargs)?)
}

fn request_param_names(params: &JsonMap) -> BTreeSet<String> {
    let mut names: BTreeSet<String> = params.keys().cloned().collect();
    names.insert(MODEL_KEY.to_owned());
    names.insert(MESSAGES_KEY.to_owned());
    names
}

// the agentic loop drops every interceptor's markers except the code interpreter's own, and its private call plumbing
fn filtered_kwargs(source: &JsonMap) -> JsonMap {
    source
        .iter()
        .filter(|(key, _)| {
            !OTHER_INTERCEPTION_PREFIXES
                .iter()
                .any(|prefix| key.starts_with(prefix))
                && !FOLLOWUP_INTERNAL_PARAMS.contains(&key.as_str())
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

// B4110's value type has no float variant, so it picks the keys, their precedence and the injected counters, and this module copies each surviving raw value
fn key_values(kwargs: &JsonMap) -> BTreeMap<String, AgenticKwargValue> {
    kwargs
        .keys()
        .map(|key| (key.clone(), AgenticKwargValue::Null))
        .collect()
}

fn kwarg_json(value: &AgenticKwargValue) -> Json {
    match value {
        AgenticKwargValue::Null => Json::Null,
        AgenticKwargValue::Boolean(flag) => Json::Bool(*flag),
        AgenticKwargValue::Integer(integer) => Json::Number(Number::from(*integer)),
        AgenticKwargValue::Text(text) => Json::Text(text.clone()),
        AgenticKwargValue::Array(items) => Json::List(items.iter().map(kwarg_json).collect()),
        AgenticKwargValue::Object(entries) => Json::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), kwarg_json(value)))
                .collect(),
        ),
    }
}

// LiteLLM folds the loop's own markers into litellm_metadata: the old entries first, then every marker the follow-up carries
fn with_metadata(kwargs: JsonMap) -> Result<JsonMap, serde_json::Error> {
    let mut metadata = match kwargs.get(LITELLM_METADATA_KEY) {
        Some(raw) => from_str::<IndexMap<String, Box<RawValue>>>(raw.get()).unwrap_or_default(),
        None => IndexMap::new(),
    };
    for (key, value) in &kwargs {
        if key.starts_with(AGENTIC_LOOP_PREFIX)
            || key.as_str() == MAX_AGENTIC_LOOPS_KEY
            || INTERCEPTION_INTERNAL_PREFIXES
                .iter()
                .any(|prefix| key.starts_with(prefix))
        {
            metadata.insert(key.clone(), value.clone());
        }
    }
    let mut merged = kwargs;
    merged.insert(LITELLM_METADATA_KEY.to_owned(), to_raw_value(&metadata)?);
    Ok(merged)
}
