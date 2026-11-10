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
use betterlite_logging::logging_success::types::SuccessRequest;
use betterlite_logging::registry::CallbackRegistry;
use betterlite_logging::worker::Callback;

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

pub trait SyncSuccessDispatch: Send + Sync {
    fn dispatch(
        &self,
        handler: &SuccessHandler<'_>,
        state: &mut LoggingState,
        correlation: &mut CorrelationContext,
        request: &SuccessRequest,
    ) -> bool;
}

pub struct LoggingSyncDispatch;

impl SyncSuccessDispatch for LoggingSyncDispatch {
    fn dispatch(
        &self,
        handler: &SuccessHandler<'_>,
        state: &mut LoggingState,
        correlation: &mut CorrelationContext,
        request: &SuccessRequest,
    ) -> bool {
        // ast-grep-ignore: no-long-names
        handle_sync_success_callbacks_for_async_calls(handler, state, correlation, request)
    }
}

pub struct CacheHitSuccessJob {
    pub handler: OwnedSuccessHandler,
    pub sync: Arc<dyn SyncSuccessDispatch>,
    pub state: LoggingState,
    pub correlation: CorrelationContext,
    pub request: SuccessRequest,
}

impl Callback for CacheHitSuccessJob {
    type Error = Infallible;

    fn name(&self) -> &'static str {
        "cache_hit_success"
    }

    async fn call(self) -> Result<(), Self::Error> {
        let Self {
            handler,
            sync,
            mut state,
            mut correlation,
            request,
        } = self;
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
        async_success_handler(&success, &mut state, &mut correlation, &request).await;
        sync.dispatch(&success, &mut state, &mut correlation, &request);
        Ok(())
    }
}
