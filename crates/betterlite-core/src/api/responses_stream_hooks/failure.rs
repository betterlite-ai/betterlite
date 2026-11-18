use std::sync::Arc;

use betterlite_error::error::ErrorClass;
use tokio::runtime::Handle;

use super::ResponsesStreamHooks;
use super::ports::{
    FAILURE_LOGGING_TASK, FailureDelivery, PortFailure, ResponsesStreamFailureTarget,
};
use super::state::ResponsesStreamHookState;
use crate::api::responses_stream_primitives::state::ResponsesStreamBaseState;
use crate::api::responses_stream_primitives::tasks::spawn_logging_task;

impl ResponsesStreamHooks<'_> {
    pub async fn handle_failure(
        &self,
        state: &mut ResponsesStreamBaseState,
        callback: &mut ResponsesStreamHookState,
        exception: &ErrorClass,
    ) {
        if state.failure_handled {
            return;
        }
        state.failure_handled = true;
        let delivery = FailureDelivery {
            exception: exception.clone(),
            traceback_exception: String::new(),
            start_time: state.start_time,
            end_time: self.clock.now(),
        };
        let Some(failure) = self.failure.as_ref().map(Arc::clone) else {
            return;
        };
        if Handle::try_current().is_ok() {
            let task_delivery = delivery.clone();
            let work = async move { run_failure_handlers(failure.as_ref(), &task_delivery).await };
            if let Some(handle) =
                spawn_logging_task(&state.pending_tasks, FAILURE_LOGGING_TASK, work)
            {
                callback.pending_logging_tasks.push(handle);
            }
            return;
        }
        // without a loop LiteLLM blocks in run_async_function, then submits the sync handler
        let _outcome = run_failure_handlers(failure.as_ref(), &delivery).await;
    }
}

async fn run_failure_handlers(
    failure: &dyn ResponsesStreamFailureTarget,
    delivery: &FailureDelivery,
) -> Result<(), PortFailure> {
    let outcome = failure.async_failure_handler(delivery).await;
    let _submission = failure.failure_handler(delivery);
    outcome
}

// _await_pending_logging: clear the tuple first, then wait for every task and never rethrow one
pub async fn await_pending_logging(callback: &mut ResponsesStreamHookState) {
    let pending = std::mem::take(&mut callback.pending_logging_tasks);
    for task in pending {
        if let Err(error) = task.await {
            tracing::debug!("a responses stream logging task did not finish: {error}");
        }
    }
}
