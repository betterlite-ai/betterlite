use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::api::file_content_stream::target::{
    ContentLogging, NewLogger, StreamIdentity,
};
use betterlite_core::api::file_stream_logging::logging::{
    CallbackFailure, FailureCall, FileStreamLogger, SuccessCall,
};
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset, Utc};

use super::super::super::error::SdkHttpError;

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The logging state a production request carries, plus the pre/post environment pre_call records into
pub(crate) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(crate) fn new() -> Result<Self, SdkHttpError> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
        Ok(Self {
            redactor,
            sink: Sink,
            now: Utc::now().fixed_offset(),
        })
    }

    // the object LiteLLM's logging class gets for a call that passed none
    pub(crate) fn state(&self, new: &NewLogger) -> Result<LoggingState, SdkHttpError> {
        let text = format!(
            r#"{{"model":"","call_type":"{}","start_time":"{}","litellm_call_id":"{}","function_id":"{}","litellm_trace_id":"trace-1"}}"#,
            new.call_type,
            new.start_time.to_rfc3339(),
            new.litellm_call_id,
            new.function_id,
        );
        let init: LoggingInit = serde_json::from_str(&text)
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        let setup = CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        };
        LoggingState::new(
            init,
            setup,
            &self.redactor,
            || String::from("generated"),
            Duration::default,
        )
        .map_err(SdkHttpError::LoggingState)
    }

    pub(crate) fn environment(&self) -> PrePostEnv<'_> {
        PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        }
    }
}

// The supplied logging object: it records what file_content_streaming wrote before the provider branch
#[derive(Debug, Default)]
pub(crate) struct RecordingLogging {
    identities: Mutex<Vec<RecordedIdentity>>,
    successes: AtomicUsize,
    failures: AtomicUsize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedIdentity {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: String,
    pub(crate) api_base: Option<String>,
}

impl RecordingLogging {
    pub(crate) fn identities(&self) -> Vec<RecordedIdentity> {
        self.identities
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ContentLogging for RecordingLogging {
    fn record_identity(&self, identity: &StreamIdentity<'_>) {
        self.identities
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedIdentity {
                model: identity.model.to_owned(),
                custom_llm_provider: identity.custom_llm_provider.to_owned(),
                api_base: identity.api_base.map(str::to_owned),
            });
    }

    fn stream_logger(self: Arc<Self>) -> Arc<dyn FileStreamLogger> {
        self
    }
}

#[async_trait]
impl FileStreamLogger for RecordingLogging {
    fn start_time(&self) -> Option<DateTime<FixedOffset>> {
        None
    }

    fn details_api_base(&self) -> Option<String> {
        None
    }

    fn build_payload(
        &self,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        None
    }

    fn success_handler(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        self.successes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn async_success_handler(&self, call: SuccessCall) -> Result<(), CallbackFailure> {
        self.success_handler(call)
    }

    fn success_sync_callbacks(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    fn failure_handler(&self, _call: FailureCall) -> Result<(), CallbackFailure> {
        self.failures.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn async_failure_handler(&self, call: FailureCall) -> Result<(), CallbackFailure> {
        self.failure_handler(call)
    }
}
