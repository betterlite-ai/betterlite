use super::error::{CallbackError, PatchError};
use super::executor::{ContinuationExecutor, PlanRun, ResponsesContinuation};
use super::hooks::{HookRun, post_response_hook, run_agentic_loop_cleanup};
use super::values::{merged_params, typed_map};
use super::{CONVERTED_STREAM_KEY, INPUT_KEY, LITELLM_LOGGING_OBJ_KEY, MODEL_KEY, STREAM_KEY};
use crate::api::responses_hook_policy::followup::{
    AgenticFollowupInput, build_agentic_followup_kwargs,
};
use betterlite_context::agentic_loop_plan::AgenticLoopRequestPatch;
use betterlite_models::decisions::Json;
use betterlite_openai::code_execution_records::OTHER_INTERCEPTION_PREFIXES;
use indexmap::IndexMap;
use serde_json::value::RawValue;
use std::collections::BTreeSet;
pub(crate) async fn execute_responses_agentic_plan<Executor>(
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
    let outcome = match executor.responses(&continuation).await {
        Ok(response) => {
            Ok(post_response_hook(&hook, response, run.plan, run.run.hook_kwargs).await)
        }
        Err(source) => Err(CallbackError::Execution(source)),
    };
    run_agentic_loop_cleanup(&hook, run.plan, run.run.hook_kwargs).await;
    outcome
}
fn build_continuation(run: &PlanRun<'_, '_>) -> Result<ResponsesContinuation, PatchError> {
    let empty = AgenticLoopRequestPatch::default();
    let patch = run.plan.request_patch.as_ref().unwrap_or(&empty);
    let Some(messages) = patch.messages.as_deref() else {
        return Err(PatchError::MissingResponsesInput);
    };
    let mut params = merged_params(run.run.call.optional_params, patch)?;
    params.shift_remove(STREAM_KEY);
    params.shift_remove(CONVERTED_STREAM_KEY);
    let kwargs = followup_kwargs(run, &params, patch)?;
    let mut model = run.run.call.model.to_owned();
    if let Some(patched) = &patch.model {
        model.clone_from(patched);
    }
    Ok(ResponsesContinuation {
        model,
        input: messages.to_vec(),
        params,
        kwargs,
    })
}
fn followup_kwargs(
    run: &PlanRun<'_, '_>,
    params: &IndexMap<String, Json>,
    patch: &AgenticLoopRequestPatch,
) -> Result<IndexMap<String, Json>, PatchError> {
    let mut request_params: BTreeSet<String> = params.keys().cloned().collect();
    request_params.insert(MODEL_KEY.to_owned());
    request_params.insert(INPUT_KEY.to_owned());
    let mut request_kwargs = IndexMap::new();
    for (key, value) in run.run.provider_kwargs {
        let internal = OTHER_INTERCEPTION_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(*prefix))
            || key.as_str() == LITELLM_LOGGING_OBJ_KEY
            || key.as_str() == CONVERTED_STREAM_KEY;
        if !internal {
            request_kwargs.insert(key.clone(), value.clone());
        }
    }
    let patch_kwargs = typed_map(&patch.kwargs)?;
    let kwargs = build_agentic_followup_kwargs(&AgenticFollowupInput {
        request_kwargs: &request_kwargs,
        patch_kwargs: &patch_kwargs,
        request_params: &request_params,
        depth: run.run.settings.depth,
        max_loops: run.run.settings.max_loops,
        fingerprints: &run.run.settings.fingerprints,
        fingerprint: run.run.fingerprint,
    })?;
    Ok(kwargs)
}
