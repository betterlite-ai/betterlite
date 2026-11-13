use super::assembled::{claim_final_stream_dispatch, is_assembled_stream_success};
use super::delivery::global_loggers;
use super::handler::{SuccessHandler, async_success_handler, success_handler};
use super::types::{PASS_THROUGH_CALL_TYPE, SuccessRequest};
use crate::dynamic_callbacks::DynamicCallbackLists;
use crate::dynamic_callbacks::filters::{
    get_combined_callback_list, remove_internal_callbacks, remove_logger_callbacks,
};
use crate::logging_state::LoggingState;
use crate::logging_state::correlation::CorrelationContext;
use crate::logging_state::events::is_sync_litellm_request;
use crate::registry::CallbackRegistry;

pub async fn dispatch_success_handlers(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    correlation: &mut CorrelationContext,
    request: &SuccessRequest,
) {
    if is_assembled_stream_success(state, &request.result) && !claim_final_stream_dispatch(state) {
        return;
    }
    let sync_sdk = is_sync_litellm_request(&state.litellm_params);
    let passthrough = state.call_type == PASS_THROUGH_CALL_TYPE;
    if sync_sdk && !request.prefer_async_handlers && !passthrough {
        success_handler(handler, state, correlation, request);
        return;
    }
    async_success_handler(handler, state, correlation, request).await;
    // ast-grep-ignore: no-long-names
    handle_sync_success_callbacks_for_async_calls(handler, state, correlation, request);
}

// ast-grep-ignore: no-long-names
pub fn handle_sync_success_callbacks_for_async_calls(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    correlation: &mut CorrelationContext,
    request: &SuccessRequest,
) -> bool {
    // ast-grep-ignore: no-long-names
    if !should_run_sync_success_callbacks_for_async_calls(
        state,
        handler.registry,
        handler.known_callback,
    ) {
        return false;
    }
    success_handler(handler, state, correlation, request);
    true
}

// LiteLLM drops every logger and every known callback name, so only legacy callables are left to run
#[must_use]
// ast-grep-ignore: no-long-names
pub fn should_run_sync_success_callbacks_for_async_calls(
    state: &LoggingState,
    registry: &CallbackRegistry,
    is_known: &(dyn Fn(&str) -> bool + Send + Sync),
) -> bool {
    let lists = DynamicCallbackLists::from(&state.dynamic_callbacks);
    let combined = get_combined_callback_list(lists.success.as_deref(), &global_loggers(registry));
    !remove_internal_callbacks(&remove_logger_callbacks(&combined, is_known)).is_empty()
}
