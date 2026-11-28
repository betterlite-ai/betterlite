use betterlite_anthropic::anthropic_http_stream::call::{StreamLogger, StreamPostCall};
pub(crate) use betterlite_logging::logging_pre_post::env::TracingSink;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, LogSink, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState, LoggingStateError};
use std::time::Duration;

use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

// The stream handler's post-call record: counts only, so no prompt or answer text reaches a log.
pub struct TracingStreamLogger;

impl StreamLogger for TracingStreamLogger {
    fn post_call(&self, call: StreamPostCall<'_>) {
        tracing::debug!(
            original_response = call.original_response,
            input_bytes = call.complete_input_dict.len(),
            "anthropic stream post-call"
        );
    }
}

// One builder for both the adapter and the fixture, so the logging call has a single source.
pub(crate) fn state<'env>(
    sink: &'env dyn LogSink,
    redactor: &'env Redactor,
    model: &str,
    call_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<(LoggingState, PrePostEnv<'env>), LoggingStateError> {
    let state = LoggingState::new(
        init(model, call_id, now),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || call_id.to_owned(),
        Duration::default,
    )?;
    Ok((state, env(redactor, sink, now)))
}

fn init(model: &str, call_id: &str, now: DateTime<FixedOffset>) -> LoggingInit {
    LoggingInit {
        model: model.to_owned(),
        call_type: String::from("completion"),
        start_time: now,
        litellm_call_id: call_id.to_owned(),
        function_id: String::from("function"),
        messages: None,
        stream: None,
        litellm_trace_id: Some(String::from("trace")),
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    }
}

fn env<'env>(
    redactor: &'env Redactor,
    sink: &'env dyn LogSink,
    now: DateTime<FixedOffset>,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}
