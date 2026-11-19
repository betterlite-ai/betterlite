use std::convert::Infallible;
use std::sync::{Arc, PoisonError};

use async_trait::async_trait;
use betterlite_logging::logging_state::events::claim_async_success_log;
use betterlite_logging::logging_success::types::SuccessResult;
use betterlite_logging::worker::{Admission, Callback, LoggingWorker};

use super::error::CallError;
use super::ports::CallTimes;
use super::state::LoggingHandle;
use crate::call_setup::correlation::CallOutcome;

pub const SUCCESS_LOG_JOB: &str = "client_async_success_log";

// The logging_obj.async_success_handler call _client_async_logging_helper hands to the B191 worker
#[derive(Clone)]
pub struct AsyncSuccessLog {
    pub logging: Arc<dyn CallLogging>,
    pub handle: LoggingHandle,
    pub result: SuccessResult,
    pub times: CallTimes,
    pub is_completion_with_fallbacks: bool,
}

impl AsyncSuccessLog {
    pub async fn deliver(self) {
        if self.is_completion_with_fallbacks {
            return;
        }
        self.logging
            .async_success(&self.handle, &self.result, self.times)
            .await;
    }
}

impl Callback for AsyncSuccessLog {
    type Error = Infallible;

    fn name(&self) -> &str {
        SUCCESS_LOG_JOB
    }

    async fn call(self) -> Result<(), Self::Error> {
        self.deliver().await;
        Ok(())
    }
}

pub trait SuccessLogScheduler: Send + Sync {
    fn schedule(&self, job: AsyncSuccessLog) -> Admission;
}

// The global worker: one queue for every background success log the async wrapper fires
pub struct WorkerScheduler {
    pub worker: Arc<LoggingWorker<AsyncSuccessLog>>,
}

impl SuccessLogScheduler for WorkerScheduler {
    fn schedule(&self, job: AsyncSuccessLog) -> Admission {
        self.worker.enqueue(job)
    }
}

// B250's success handlers and B251's failure handlers, entered the way LiteLLM's logging object is
#[async_trait]
pub trait CallLogging: Send + Sync {
    fn success(&self, handle: &LoggingHandle, result: SuccessResult, times: CallTimes);

    async fn async_success(&self, handle: &LoggingHandle, result: SuccessResult, times: CallTimes);

    // ast-grep-ignore: no-long-names
    fn sync_success_callbacks_for_async_calls(
        &self,
        handle: &LoggingHandle,
        result: SuccessResult,
        times: CallTimes,
    );

    fn failure_sync(&self, handle: &LoggingHandle, error: &CallError, times: CallTimes);

    async fn failure_async(&self, handle: &LoggingHandle, error: &CallError, times: CallTimes);
}

#[derive(Debug, Clone, Copy)]
pub struct DispatchFlags {
    pub is_completion_with_fallbacks: bool,
    pub is_internal_call: bool,
}

// the result, the times and the flags one success log carries through the dispatch
pub struct SuccessLogInput {
    pub result: SuccessResult,
    pub times: CallTimes,
    pub flags: DispatchFlags,
}

#[must_use]
pub fn success_result(outcome: &CallOutcome, call_type: &str) -> SuccessResult {
    match outcome {
        CallOutcome::Response(response) if matches!(call_type, "completion" | "acompletion") => {
            SuccessResult::Chat(response.clone())
        }
        CallOutcome::Response(response) if matches!(call_type, "embedding" | "aembedding") => {
            SuccessResult::Embeddings(response.clone())
        }
        CallOutcome::Response(response) => SuccessResult::Other(response.clone()),
        CallOutcome::ChatStream(stream) => SuccessResult::Chunk(stream.model_response.clone()),
        CallOutcome::ResponsesStream(response) => SuccessResult::Other(response.clone()),
    }
}

// _dispatch_success_logging: the async log is scheduled (or parked), and the sync callbacks run here
pub fn dispatch_success_logging(
    logging: &Arc<dyn CallLogging>,
    scheduler: &Arc<dyn SuccessLogScheduler>,
    handle: &LoggingHandle,
    input: SuccessLogInput,
) {
    if !input.flags.is_internal_call {
        schedule_async_success_logging(logging, scheduler, handle, &input);
    }
    logging.sync_success_callbacks_for_async_calls(handle, input.result, input.times);
}

// _schedule_async_success_logging: the innermost wrapper claims the log, and a deferred one waits in its slot
fn schedule_async_success_logging(
    logging: &Arc<dyn CallLogging>,
    scheduler: &Arc<dyn SuccessLogScheduler>,
    handle: &LoggingHandle,
    input: &SuccessLogInput,
) {
    let defer = {
        let mut state = handle.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !claim_async_success_log(&mut state) {
            return;
        }
        state.deferred.defer_async_logging
    };
    let job = AsyncSuccessLog {
        logging: Arc::clone(logging),
        handle: handle.clone(),
        result: input.result.clone(),
        times: input.times,
        is_completion_with_fallbacks: input.flags.is_completion_with_fallbacks,
    };
    if !defer {
        warn_rejected(scheduler.schedule(job));
        return;
    }
    let mut slot = handle
        .deferred
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if slot.is_none() {
        *slot = Some(job);
    }
}

fn warn_rejected(admission: Admission) {
    if let Admission::Rejected(reason) = admission {
        tracing::warn!(
            ?reason,
            "the async success log was dropped: the logging worker rejected it"
        );
    }
}
