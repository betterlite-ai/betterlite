use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use super::error::PrePostError;
use super::hooks::{InputCallback, SentrySink, redact_callback_fields};
use crate::logging_privacy::request::Decision;
use crate::logging_privacy::snapshot::callback_snapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRecord {
    pub level: LogLevel,
    pub message: String,
    pub fields: IndexMap<String, Json>,
}

pub trait LogSink {
    fn emit(&self, record: LogRecord);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DebugSettings {
    pub verbose: bool,
    pub json_logs: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GlobalFlags {
    pub log_raw_request_response: bool,
    pub turn_off_message_logging: bool,
}

pub type LoggerFn<'hook> = &'hook dyn Fn(&Json) -> Result<(), PrePostError>;

pub struct PrePostEnv<'env> {
    pub debug: DebugSettings,
    pub globals: GlobalFlags,
    pub decision: Decision,
    pub redactor: &'env Redactor,
    pub log: &'env dyn LogSink,
    pub input_callbacks: &'env [InputCallback<'env>],
    pub sentry: Option<&'env dyn SentrySink>,
    pub logger_fn: Option<LoggerFn<'env>>,
    pub now: DateTime<FixedOffset>,
}

impl PrePostEnv<'_> {
    // Every record passes the credential scrubber here, so no caller can forget it
    pub fn emit(&self, level: LogLevel, message: &str, fields: IndexMap<String, Json>) {
        self.log.emit(LogRecord {
            level,
            message: self.redactor.redact_string(message),
            fields,
        });
    }

    pub fn report_failure(&self, headline: &str, error: &PrePostError) {
        self.emit(
            LogLevel::Error,
            &format!("{headline} {error}"),
            IndexMap::new(),
        );
        if let Some(sentry) = self.sentry {
            sentry.capture_exception(&error.to_string());
        }
    }

    // logger_fn failures are logged without a sentry capture, as in LiteLLM
    pub fn run_logger_fn(&self, details: &IndexMap<String, Json>) {
        let Some(logger_fn) = self.logger_fn else {
            return;
        };
        let mut snapshot =
            callback_snapshot(Json::Object(details.clone()), self.decision, self.redactor);
        redact_callback_fields(&mut snapshot, self.decision);
        if let Err(error) = logger_fn(&snapshot) {
            self.emit(
                LogLevel::Error,
                &format!(
                    "LiteLLM.LoggingError: [Non-Blocking] Exception occurred while logging {error}"
                ),
                IndexMap::new(),
            );
        }
    }
}
