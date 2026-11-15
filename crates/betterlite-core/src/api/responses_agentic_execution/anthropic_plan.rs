use betterlite_context::agentic_loop_plan::{AgenticLoopRequestPatch, JsonMap};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use serde_json::value::RawValue;

use super::error::{CallbackError, PatchError};
use super::executor::{AnthropicContinuation, CallbackRun, ContinuationExecutor, PlanRun};
use super::hooks::{HookRun, post_response_hook};
use super::values::{
    count_field, flag_field, json_items, merged_params, message_list, take_or, text_field,
};
use super::{
    AGENTIC_LOOP_DEPTH_KEY, AGENTIC_LOOP_FINGERPRINTS_KEY, DEFAULT_MAX_TOKENS,
    LITELLM_LOGGING_OBJ_KEY, MAX_AGENTIC_LOOPS_KEY, MAX_TOKENS_KEY, MESSAGES_KEY, MODEL_KEY,
    STREAM_KEY,
};
use crate::api::responses_hook_policy::error::AgenticFollowupKwargsError;

// _execute_anthropic_agentic_plan drops only these two interceptor prefixes, where the responses plan drops three
const DROPPED_PREFIXES: [&str; 2] = ["_websearch_interception", "_compression_interception"];

// _execute_anthropic_agentic_plan: no cleanup finally here, and the post hook keeps a failed send's error
pub(crate) async fn execute_anthropic_agentic_plan<Executor>(
    run: &PlanRun<'_, '_>,
    executor: &Executor,
) -> Result<Box<RawValue>, CallbackError<Executor::Error>>
where
    Executor: ContinuationExecutor + ?Sized,
{
    let continuation = build_continuation(run)?;
    let hook = HookRun {
        callback: run.run.callback,
        logging: run.run.call.logging,
        model: run.run.call.model,
    };
    let response = executor
        .anthropic_messages(&continuation)
        .await
        .map_err(CallbackError::Execution)?;
    Ok(post_response_hook(&hook, response, run.plan, run.run.hook_kwargs).await)
}

// the dict LiteLLM passes to acreate: named fields first, then the optional params, then the kwargs
fn build_continuation(run: &PlanRun<'_, '_>) -> Result<AnthropicContinuation, PatchError> {
    let empty = AgenticLoopRequestPatch::default();
    let patch = run.plan.request_patch.as_ref().unwrap_or(&empty);
    let Some(messages) = patch.messages.as_deref() else {
        return Err(PatchError::MissingMessages);
    };
    let model = followup_model(patch, run.run)?;
    let mut params = merged_params(run.run.call.optional_params, patch)?;
    let max_tokens = max_tokens(patch, &mut params, run.run.provider_kwargs)?;
    let followup = followup_kwargs(run.run, &params, patch)?;
    let mut extras = params;
    extras.extend(followup);
    let model = text_field(
        take_or(&mut extras, MODEL_KEY, Json::Text(model)),
        MODEL_KEY,
    )?;
    let messages = message_list(
        take_or(&mut extras, MESSAGES_KEY, json_items(messages)?),
        MESSAGES_KEY,
    )?;
    let stream = flag_field(
        take_or(&mut extras, STREAM_KEY, Json::Bool(run.run.call.stream)),
        STREAM_KEY,
    )?;
    let max_tokens = count_field(
        take_or(
            &mut extras,
            MAX_TOKENS_KEY,
            Json::Number(Number::from(max_tokens)),
        ),
        MAX_TOKENS_KEY,
    )?;
    Ok(AnthropicContinuation {
        model,
        messages,
        max_tokens,
        stream,
        params: extras,
    })
}

// patch.model wins over the model logging recorded, which wins over the request's own model
fn followup_model(
    patch: &AgenticLoopRequestPatch,
    run: &CallbackRun<'_>,
) -> Result<String, PatchError> {
    let params = run.call.logging.and_then(|view| view.agentic_loop_params);
    let recorded = params.map_or(Ok(Json::Null), recorded_model)?;
    let mut logged = match recorded {
        Json::Null => run.call.model.to_owned(),
        value => text_field(value, MODEL_KEY)?,
    };
    if let Some(patched) = &patch.model {
        logged = patched.clone();
    }
    Ok(logged)
}

fn recorded_model(params: &JsonMap) -> Result<Json, serde_json::Error> {
    params
        .get(MODEL_KEY)
        .map_or(Ok(Json::Null), |raw| serde_json::from_str(raw.get()))
}

// the computed value comes first; a kwargs max_tokens still overrides it when the mapping is merged
fn max_tokens(
    patch: &AgenticLoopRequestPatch,
    params: &mut IndexMap<String, Json>,
    kwargs: &IndexMap<String, Json>,
) -> Result<u64, PatchError> {
    let popped = params.shift_remove(MAX_TOKENS_KEY);
    if let Some(patched) = patch.max_tokens {
        return Ok(patched);
    }
    let optional = match popped {
        None | Some(Json::Null) => None,
        Some(value) => Some(count_field(value, MAX_TOKENS_KEY)?),
    };
    let followup = match kwargs.get(MAX_TOKENS_KEY) {
        None | Some(Json::Null) => None,
        Some(value) => Some(count_field(value.clone(), MAX_TOKENS_KEY)?),
    };
    match (optional, followup) {
        (Some(optional), _) => Ok(optional),
        (None, Some(followup)) => Ok(followup),
        (None, None) => Ok(DEFAULT_MAX_TOKENS),
    }
}

fn followup_kwargs(
    run: &CallbackRun<'_>,
    params: &IndexMap<String, Json>,
    patch: &AgenticLoopRequestPatch,
) -> Result<IndexMap<String, Json>, PatchError> {
    let mut kwargs = IndexMap::new();
    for (key, value) in run.provider_kwargs {
        let internal = DROPPED_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(*prefix))
            || key.as_str() == LITELLM_LOGGING_OBJ_KEY
            || params.contains_key(key);
        if !internal {
            kwargs.insert(key.clone(), value.clone());
        }
    }
    for (key, value) in &patch.kwargs {
        kwargs.insert(key.clone(), serde_json::from_str(value.get())?);
    }
    let depth = run
        .settings
        .depth
        .checked_add(1)
        .ok_or(AgenticFollowupKwargsError::DepthOverflow)?;
    kwargs.insert(
        AGENTIC_LOOP_DEPTH_KEY.to_owned(),
        Json::Number(Number::from(depth)),
    );
    kwargs.insert(
        MAX_AGENTIC_LOOPS_KEY.to_owned(),
        Json::Number(Number::from(run.settings.max_loops)),
    );
    let mut fingerprints = run.settings.fingerprints.clone();
    fingerprints.push(Json::Text(run.fingerprint.to_owned()));
    kwargs.insert(
        AGENTIC_LOOP_FINGERPRINTS_KEY.to_owned(),
        Json::List(fingerprints),
    );
    Ok(kwargs)
}
