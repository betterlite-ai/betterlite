use std::convert::Infallible;
use std::sync::Arc;

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::CorrelationContext;
// ast-grep-ignore: no-long-names
use betterlite_logging::logging_success::dispatch::handle_sync_success_callbacks_for_async_calls;
use betterlite_logging::logging_success::handler::{SuccessHandler, async_success_handler};
use betterlite_logging::logging_success::owned::OwnedSuccessHandler;
use betterlite_logging::logging_success::types::{SuccessRequest, SuccessResult};
use betterlite_logging::worker::{Admission, Callback, LoggingWorker};
use chrono::{DateTime, FixedOffset};

use super::context::{CacheHitContext, CacheHitError, record_cache_hit_context};

pub struct CacheHitCallbacks {
    pub handler: Arc<OwnedSuccessHandler>,
    pub state: LoggingState,
    pub correlation: CorrelationContext,
    pub result: SuccessResult,
    pub start_time: Option<DateTime<FixedOffset>>,
    pub end_time: Option<DateTime<FixedOffset>>,
}

pub struct CacheHitSuccessJob {
    pub handler: Arc<OwnedSuccessHandler>,
    pub state: LoggingState,
    pub correlation: CorrelationContext,
    pub request: SuccessRequest,
}

pub fn log_cache_hit_on_callbacks(
    worker: &LoggingWorker<CacheHitSuccessJob>,
    hit: &CacheHitContext<'_>,
    callbacks: CacheHitCallbacks,
) -> Result<Admission, CacheHitError> {
    let CacheHitCallbacks {
        handler,
        mut state,
        correlation,
        result,
        start_time,
        end_time,
    } = callbacks;
    record_cache_hit_context(&mut state, hit)?;
    let request = SuccessRequest {
        result,
        start_time,
        end_time,
        cache_hit: Some(true),
        prefer_async_handlers: false,
        standard_logging_object: None,
        guardrail_snapshot: None,
    };
    Ok(worker.enqueue(CacheHitSuccessJob {
        handler,
        state,
        correlation,
        request,
    }))
}

pub async fn run_cache_hit_success(
    handler: &OwnedSuccessHandler,
    state: &mut LoggingState,
    correlation: &mut CorrelationContext,
    request: &SuccessRequest,
) {
    let success = SuccessHandler::from(handler);
    async_success_handler(&success, state, correlation, request).await;
    // ast-grep-ignore: no-long-names
    handle_sync_success_callbacks_for_async_calls(&success, state, correlation, request);
}

impl Callback for CacheHitSuccessJob {
    type Error = Infallible;

    fn name(&self) -> &'static str {
        "cache_hit_success"
    }

    async fn call(self) -> Result<(), Self::Error> {
        let Self {
            handler,
            mut state,
            mut correlation,
            request,
        } = self;
        run_cache_hit_success(handler.as_ref(), &mut state, &mut correlation, &request).await;
        Ok(())
    }
}
