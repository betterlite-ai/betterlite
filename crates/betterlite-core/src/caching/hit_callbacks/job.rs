use std::convert::Infallible;
use std::sync::Arc;

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_logging::exporter_options::failure_counter::CallbackFailureCounter;
use betterlite_logging::exporter_options::redaction::ResponseStamp;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::CorrelationContext;
// ast-grep-ignore: no-long-names
use betterlite_logging::logging_success::dispatch::handle_sync_success_callbacks_for_async_calls;
use betterlite_logging::logging_success::handler::{SuccessHandler, async_success_handler};
use betterlite_logging::logging_success::ports::{
    StandardPayloadBuilder, StreamAssembler, SuccessClock,
};
use betterlite_logging::logging_success::types::{SuccessRequest, SuccessResult};
use betterlite_logging::registry::CallbackRegistry;
use betterlite_logging::worker::{Admission, Callback, LoggingWorker};
use chrono::{DateTime, FixedOffset};

use super::context::{CacheHitContext, CacheHitError, record_cache_hit_context};

// Everything B250's SuccessHandler borrows, owned so a cache-hit job outlives the request that queued it
pub struct OwnedSuccessHandler {
    pub registry: Arc<CallbackRegistry>,
    pub payload_builder: Arc<dyn StandardPayloadBuilder>,
    pub stream_assembler: Arc<dyn StreamAssembler>,
    pub clock: Arc<dyn SuccessClock>,
    pub failure_counters: Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
    pub excluded_fields: Vec<String>,
    pub known_callback: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    pub stamp: ResponseStamp,
    pub privacy: Settings,
}

pub struct CacheHitCallbacks {
    pub handler: OwnedSuccessHandler,
    pub state: LoggingState,
    pub correlation: CorrelationContext,
    pub result: SuccessResult,
    pub start_time: Option<DateTime<FixedOffset>>,
    pub end_time: Option<DateTime<FixedOffset>>,
}

pub struct CacheHitSuccessJob {
    pub handler: OwnedSuccessHandler,
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
    let success = SuccessHandler {
        registry: &handler.registry,
        payload_builder: handler.payload_builder.as_ref(),
        stream_assembler: handler.stream_assembler.as_ref(),
        clock: handler.clock.as_ref(),
        failure_counters: &handler.failure_counters,
        excluded_fields: &handler.excluded_fields,
        known_callback: handler.known_callback.as_ref(),
        stamp: &handler.stamp,
        privacy: handler.privacy,
    };
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
        run_cache_hit_success(&handler, &mut state, &mut correlation, &request).await;
        Ok(())
    }
}
