use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;

use super::seams::{LoggingFuture, ReportedFailure};

// Python's _on_deferred_stream_complete, called with the parked arguments once the stream ends
pub type DeferredCallback = Box<dyn FnOnce(Vec<LoggingFuture>) -> LoggingFuture + Send>;

// Python's _on_detached_stream_failure
#[async_trait]
pub trait DetachedFailureHook: Send + Sync {
    async fn on_detached_failure(&self, exception: &ReportedFailure)
    -> Result<(), ReportedFailure>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookFailure {
    // the proxy's HTTPException, which has no client left to reach
    Http,
    Other(ReportedFailure),
}

// The proxy's post_call_failure_hook with the key and request data it was armed with
#[async_trait]
pub trait PostCallFailureHook: Send + Sync {
    async fn post_call_failure(
        &self,
        original_exception: &ReportedFailure,
    ) -> Result<(), HookFailure>;
}

struct ArmedHook {
    post_call: Arc<dyn PostCallFailureHook>,
}

#[async_trait]
impl DetachedFailureHook for ArmedHook {
    async fn on_detached_failure(
        &self,
        exception: &ReportedFailure,
    ) -> Result<(), ReportedFailure> {
        match self.post_call.post_call_failure(exception).await {
            Ok(()) | Err(HookFailure::Http) => Ok(()),
            Err(HookFailure::Other(failure)) => Err(failure),
        }
    }
}

#[derive(Default)]
pub struct Fields {
    pub deferred_callback: Option<DeferredCallback>,
    pub deferred_args: Option<Vec<LoggingFuture>>,
    pub detached_failure_hook: Option<Arc<dyn DetachedFailureHook>>,
}

// The per-request slots LiteLLM hangs on its logging object; B247's LoggingState has no field for them
#[derive(Default)]
pub struct StreamSlots {
    pub fields: Mutex<Fields>,
}

impl StreamSlots {
    // Both fields or neither: a missing one leaves the slots unchanged
    pub fn take_pending(&self) -> Option<(DeferredCallback, Vec<LoggingFuture>)> {
        let mut fields = self.fields.lock().unwrap_or_else(PoisonError::into_inner);
        if fields.deferred_callback.is_none() || fields.deferred_args.is_none() {
            return None;
        }
        fields
            .deferred_callback
            .take()
            .zip(fields.deferred_args.take())
    }

    // Python's _arm_detached_stream_failure_hook: a failure after the client left still reaches post_call_failure_hook
    pub fn arm_failure_hook(&self, post_call: Arc<dyn PostCallFailureHook>) {
        let hook: Arc<dyn DetachedFailureHook> = Arc::new(ArmedHook { post_call });
        self.fields
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .detached_failure_hook = Some(hook);
    }
}
