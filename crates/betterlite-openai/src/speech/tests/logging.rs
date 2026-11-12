use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};

#[derive(Default)]
pub(super) struct Sink {
    pub(super) records: Mutex<Vec<LogRecord>>,
}

impl LogSink for Sink {
    fn emit(&self, record: LogRecord) {
        self.records
            .lock()
            .expect("the sink lock is not poisoned")
            .push(record);
    }
}

pub(super) fn state() -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"tts-1","call_type":"aspeech","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the logging fixture is valid");
    let redactor = redactor();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        || Duration::default(),
    )
    .expect("the logging fixture builds")
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds")
}

pub(super) fn env<'env>(
    now: DateTime<FixedOffset>,
    redactor: &'env Redactor,
    sink: &'env Sink,
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
