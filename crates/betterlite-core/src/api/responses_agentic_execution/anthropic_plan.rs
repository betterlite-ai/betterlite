use super::error::{CallbackError, PatchError};
use super::executor::{AnthropicContinuation, CallbackRun, ContinuationExecutor, PlanRun};
use super::hooks::{HookRun, post_response_hook};
use super::values::{count_field, json_items, merged_params, message_list, take_or, text_field};
use super::{
    AGENTIC_LOOP_DEPTH_KEY, AGENTIC_LOOP_FINGERPRINTS_KEY, DEFAULT_MAX_TOKENS,
    LITELLM_LOGGING_OBJ_KEY, MAX_AGENTIC_LOOPS_KEY, MAX_TOKENS_KEY, MESSAGES_KEY, MODEL_KEY,
    STREAM_KEY,
};
use crate::api::responses_hook_policy::error::AgenticFollowupKwargsError;
use betterlite_context::agentic_loop_plan::AgenticLoopRequestPatch;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use serde_json::value::RawValue;
const DROPPED_PREFIXES: [&str; 2] = ["_websearch_interception", "_compression_interception"];
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
fn build_continuation(run: &PlanRun<'_, '_>) -> Result<AnthropicContinuation, PatchError> {
    let empty = AgenticLoopRequestPatch::default();
    let patch = run.plan.request_patch.as_ref().unwrap_or(&empty);
    let Some(messages) = patch.messages.as_deref() else {
        return Err(PatchError::MissingMessages);
    };
    let recorded_model = followup_model(patch, run.run)?;
    let mut params = merged_params(run.run.call.optional_params, patch)?;
    let computed = max_tokens(patch, &mut params, run.run.provider_kwargs)?;
    let followup = followup_kwargs(run.run, &params, patch)?;
    let mut extras = params;
    extras.extend(followup);
    let raw_model = take_or(&mut extras, MODEL_KEY, Json::Text(recorded_model));
    let model = text_field(raw_model, MODEL_KEY)?;
    let raw_messages = take_or(&mut extras, MESSAGES_KEY, json_items(messages)?);
    let continuation_messages = message_list(raw_messages, MESSAGES_KEY)?;
    let raw_stream = take_or(&mut extras, STREAM_KEY, Json::Bool(run.run.call.stream));
    let Json::Bool(stream) = raw_stream else {
        return Err(PatchError::Field(Box::new((STREAM_KEY, "a boolean"))));
    };
    let fallback = Json::Number(Number::from(computed));
    let raw_max_tokens = take_or(&mut extras, MAX_TOKENS_KEY, fallback);
    let max_tokens = count_field(raw_max_tokens, MAX_TOKENS_KEY)?;
    Ok(AnthropicContinuation {
        model,
        messages: continuation_messages,
        max_tokens,
        stream,
        params: extras,
    })
}
fn followup_model(
    patch: &AgenticLoopRequestPatch,
    run: &CallbackRun<'_>,
) -> Result<String, PatchError> {
    let logging = run.call.logging.and_then(|view| view.agentic_loop_params);
    let recorded = match logging.and_then(|params| params.get(MODEL_KEY)) {
        None => Json::Null,
        Some(raw) => serde_json::from_str(raw.get())?,
    };
    let mut logged = match recorded {
        Json::Null => run.call.model.to_owned(),
        value @ (Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_)) => text_field(value, MODEL_KEY)?,
    };
    if let Some(patched) = &patch.model {
        logged.clone_from(patched);
    }
    Ok(logged)
}
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
        (Some(value), _) | (None, Some(value)) => Ok(value),
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
