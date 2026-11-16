use super::LoggingView;
use super::logged_call_id;
use betterlite_context::agentic_callback::AgenticCallback;
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, JsonMap};
use serde_json::value::RawValue;
pub(crate) fn hook_message(error: &AgenticHookError) -> &str {
    match error {
        AgenticHookError::Failed { message, .. } => message,
    }
}
pub(crate) struct HookRun<'hook> {
    pub(crate) callback: &'hook dyn AgenticCallback,
    pub(crate) logging: Option<LoggingView<'hook>>,
    pub(crate) model: &'hook str,
}
pub(crate) async fn post_response_hook(
    hook: &HookRun<'_>,
    response: Box<RawValue>,
    plan: &AgenticLoopPlan,
    kwargs: &JsonMap,
) -> Box<RawValue> {
    match hook
        .callback
        .post_agentic_loop_response(response.clone(), plan, kwargs)
        .await
    {
        Ok(hooked) => hooked,
        Err(error) => {
            let call_id = logged_call_id(hook.logging);
            let model = hook.model;
            let message = hook_message(&error);
            tracing::error!(
                "LiteLLM.AgenticHookError: Exception in async_post_agentic_loop_response_hook [call_id={call_id} model={model}]: {message}"
            );
            response
        }
    }
}
pub(crate) async fn run_agentic_loop_cleanup(
    hook: &HookRun<'_>,
    plan: &AgenticLoopPlan,
    kwargs: &JsonMap,
) {
    if let Err(error) = hook.callback.agentic_loop_cleanup(plan, kwargs).await {
        let call_id = logged_call_id(hook.logging);
        let model = hook.model;
        let message = hook_message(&error);
        tracing::error!(
            "LiteLLM.AgenticHookError: Exception in async_agentic_loop_cleanup_hook [call_id={call_id} model={model}]: {message}"
        );
    }
}
