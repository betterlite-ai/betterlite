use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use indexmap::IndexMap;

use crate::callback_logger::CallbackLogger;
use crate::exporter_options::failure_counter::{CallbackFailureCounter, handle_callback_failure};
use crate::exporter_options::redaction::ResponseStamp;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_failure::callbacks::FailureCallbacks;
use crate::logging_failure::events::FailureDelivery;
use crate::logging_failure::helper_fn::failure_handler_helper_fn;
use crate::logging_failure::request_redaction::{RequestRedaction, request_redacts_messages};
use crate::logging_failure::special::special_failure_handlers;
use crate::logging_failure::{
    FAILED_REQUEST_REASON, FailureCall, FailureDeps, FailureReport, PASS_THROUGH_CALL_TYPE,
    SyncAdmission,
};
use crate::logging_metadata::params::metadata_from_litellm_params;
use crate::logging_state::LoggingState;
use crate::logging_state::callbacks::{CallbackKind, should_run_callback};
use crate::logging_state::correlation::restore_correlation_context;
use crate::logging_state::events::{
    LoggingEvent, has_run_logging, is_sync_litellm_request, should_run_logging,
};

pub fn failure_handler(
    log: &mut FailureCall<'_>,
    failure: &FailureReport,
    loggers: &FailureCallbacks<'_>,
    dependencies: &FailureDeps<'_>,
) {
    if prepare_failure(log, failure, dependencies, LoggingEvent::SyncFailure) {
        deliver_sync_loggers(log, loggers, dependencies);
    }
    restore_correlation_context(log.state, log.context);
}

fn prepare_failure(
    call: &mut FailureCall<'_>,
    report: &FailureReport,
    deps: &FailureDeps<'_>,
    event: LoggingEvent,
) -> bool {
    if !should_run_logging(call.state, event) {
        return false;
    }
    if let Err(error) = failure_handler_helper_fn(call, report, deps) {
        tracing::error!(%error, "the failure details could not be prepared");
        return false;
    }
    has_run_logging(call.state, event);
    true
}

fn deliver_sync_loggers(
    call: &FailureCall<'_>,
    callbacks: &FailureCallbacks<'_>,
    deps: &FailureDeps<'_>,
) {
    let Some(delivery) = deliverable(call, deps) else {
        return;
    };
    if !is_sync_litellm_request(&call.state.litellm_params)
        || call.state.call_type == PASS_THROUGH_CALL_TYPE
    {
        return;
    }
    for logger in callbacks.sync_loggers() {
        if logger_may_run(call.state, logger.as_ref()) {
            delivery.deliver_sync(logger.as_ref());
        }
    }
}

fn logger_may_run(state: &LoggingState, logger: &dyn CallbackLogger) -> bool {
    let kind = if logger.callback_name().contains("_PROXY_") {
        CallbackKind::ProxyInternal
    } else {
        CallbackKind::External
    };
    should_run_callback(state, kind, &state.litellm_params)
}

pub async fn async_failure_handler(
    call: &mut FailureCall<'_>,
    report: &FailureReport,
    callbacks: &FailureCallbacks<'_>,
    deps: &FailureDeps<'_>,
) {
    if let Some(baseline) = deps.baseline
        && let Err(error) = baseline.invalidate_baseline(FAILED_REQUEST_REASON).await
    {
        tracing::warn!(%error, "the baseline cache estimate could not be invalidated");
    }
    special_failure_handlers(call.state, report, deps.special_listeners).await;
    if prepare_failure(call, report, deps, LoggingEvent::AsyncFailure) {
        if let Some(delivery) = deliverable(call, deps) {
            let mut loggers = callbacks.async_loggers();
            loggers.retain(|logger| logger_may_run(call.state, logger.as_ref()));
            for logger in loggers {
                deliver_async_logger(&delivery, logger.as_ref(), &deps.counters).await;
            }
        }
        if let Some(flush) = deps.flush
            && let Err(error) = flush.flush_post_call().await
        {
            tracing::warn!(%error, "the post-call redis batches could not be flushed");
        }
    }
    restore_correlation_context(call.state, call.context);
}

pub async fn dispatch_failure_handlers(
    call: &mut FailureCall<'_>,
    report: &FailureReport,
    callbacks: &FailureCallbacks<'_>,
    deps: &FailureDeps<'_>,
    prefer_async_handlers: bool,
) {
    let sync_sdk = is_sync_litellm_request(&call.state.litellm_params);
    let passthrough = call.state.call_type == PASS_THROUGH_CALL_TYPE;
    if sync_sdk && !prefer_async_handlers && !passthrough {
        failure_handler(call, report, callbacks, deps);
        return;
    }
    async_failure_handler(call, report, callbacks, deps).await;
    if !callbacks.should_run_sync_failure_callbacks_for_async_calls()
        || !should_run_logging(call.state, LoggingEvent::SyncFailure)
    {
        return;
    }
    let job = SyncFailureJob::prepare(call, callbacks, deps);
    if deps.executor.enqueue_sync_failure(job) == SyncAdmission::Accepted {
        has_run_logging(call.state, LoggingEvent::SyncFailure);
    } else {
        tracing::warn!("the queued sync failure callbacks were dropped: the queue is full");
    }
}

fn deliverable<'call>(
    call: &'call FailureCall<'_>,
    deps: &'call FailureDeps<'_>,
) -> Option<FailureDelivery<'call>> {
    let Some(payload) = call.record.payload.as_ref() else {
        tracing::warn!(
            "the failure callbacks were skipped: the standard logging payload could not be built"
        );
        return None;
    };
    Some(FailureDelivery {
        payload,
        dynamic_params: Arc::new(Json::Object(call.state.callback_dynamic_params.clone())),
        litellm_metadata: Arc::new(metadata_from_litellm_params(&call.state.litellm_params)),
        stream: call.state.stream == Some(true),
        excluded_fields: deps.excluded_fields,
        stamp: deps.stamp.clone(),
        request_redaction: request_redacts_messages(call.state, deps.privacy),
    })
}

async fn deliver_async_logger(
    delivery: &FailureDelivery<'_>,
    logger: &dyn CallbackLogger,
    counters: &[&(dyn CallbackFailureCounter + Send + Sync)],
) {
    match delivery.event_for(logger) {
        Ok(event) => {
            if let Err(error) = logger.async_log_failure_event(Arc::new(event)).await {
                tracing::error!(%error, "an async failure callback errored");
                count_callback_failure(counters, logger.callback_name());
            }
        }
        Err(error) => {
            tracing::error!(
                %error,
                callback = logger.callback_name(),
                "the failure payload could not be redacted for a callback"
            );
            count_callback_failure(counters, logger.callback_name());
        }
    }
}

fn count_callback_failure(
    counters: &[&(dyn CallbackFailureCounter + Send + Sync)],
    callback_name: &str,
) {
    let mut widened: Vec<&dyn CallbackFailureCounter> = Vec::with_capacity(counters.len());
    for counter in counters {
        widened.push(*counter);
    }
    handle_callback_failure(callback_name, &widened);
}

pub struct SyncFailureJob {
    pub litellm_params: IndexMap<String, Json>,
    pub call_type: String,
    pub stream: bool,
    pub dynamic_params: Arc<StandardCallbackDynamicParams>,
    pub loggers: Vec<Arc<dyn CallbackLogger>>,
    pub payload: Option<Arc<StandardLoggingPayload>>,
    pub excluded_fields: Vec<String>,
    pub stamp: ResponseStamp,
    pub request_redaction: RequestRedaction,
}

impl SyncFailureJob {
    #[must_use]
    pub fn prepare(
        call: &FailureCall<'_>,
        callbacks: &FailureCallbacks<'_>,
        deps: &FailureDeps<'_>,
    ) -> Self {
        let mut loggers = callbacks.sync_loggers();
        loggers.retain(|logger| logger_may_run(call.state, logger.as_ref()));
        Self {
            litellm_params: call.state.litellm_params.clone(),
            call_type: call.state.call_type.clone(),
            stream: call.state.stream == Some(true),
            dynamic_params: Arc::new(Json::Object(call.state.callback_dynamic_params.clone())),
            loggers,
            payload: call.record.payload.as_ref().map(Arc::clone),
            excluded_fields: deps.excluded_fields.to_vec(),
            stamp: deps.stamp.clone(),
            request_redaction: request_redacts_messages(call.state, deps.privacy),
        }
    }
}

pub fn run_sync_failure(job: SyncFailureJob) {
    if !is_sync_litellm_request(&job.litellm_params) || job.call_type == PASS_THROUGH_CALL_TYPE {
        return;
    }
    let Some(payload) = job.payload else {
        tracing::warn!(
            "the queued sync failure callbacks were skipped: the standard logging payload could not be built"
        );
        return;
    };
    let delivery = FailureDelivery {
        payload: &payload,
        dynamic_params: job.dynamic_params,
        litellm_metadata: Arc::new(metadata_from_litellm_params(&job.litellm_params)),
        stream: job.stream,
        excluded_fields: &job.excluded_fields,
        stamp: job.stamp,
        request_redaction: job.request_redaction,
    };
    for logger in &job.loggers {
        delivery.deliver_sync(logger.as_ref());
    }
}
