use betterlite_context::agentic_callback::AgenticCallback;
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, JsonMap};
use serde_json::value::RawValue;

use super::LoggingView;
use super::logged_call_id;

// LiteLLM logs str(e) here, which is the exception's own text before it reached our typed hook error
pub(crate) fn hook_message(error: &AgenticHookError) -> &str {
    match error {
        AgenticHookError::Failed { message, .. } => message,
    }
}

// who a post or cleanup hook belongs to, bundled so both hooks take the same three pieces
pub(crate) struct HookRun<'hook> {
    pub(crate) callback: &'hook dyn AgenticCallback,
    pub(crate) logging: Option<LoggingView<'hook>>,
    pub(crate) model: &'hook str,
}

// async_post_agentic_loop_response_hook: an error is logged and the loop keeps the provider's answer
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

// async_agentic_loop_cleanup_hook: cleanup can never replace the loop's answer or its failure
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
