#![cfg(test)]

use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::FixedOffset;

use crate::database_diagnostics::initialization::{DiagnosticFormat, DiagnosticLogger};
use crate::database_diagnostics::level::LogLevel;
use crate::database_diagnostics::message::LogArguments;
use crate::database_diagnostics::record::LogRecord;
use crate::database_diagnostics::timestamp::FixedLocalTime;

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the fixture redactor builds")
}

pub(crate) fn utc() -> FixedLocalTime {
    FixedLocalTime {
        offset: FixedOffset::east_opt(0).expect("UTC is a valid offset"),
    }
}

pub(crate) fn at_offset(seconds: i32) -> FixedLocalTime {
    FixedLocalTime {
        offset: FixedOffset::east_opt(seconds).expect("the fixture offset is valid"),
    }
}

pub(crate) fn record<'record>(
    created: f64,
    level: &'record str,
    message: &'record str,
) -> LogRecord<'record> {
    LogRecord {
        created,
        level,
        message,
        arguments: LogArguments::None,
        exception: None,
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JsonOutput {
    pub(crate) message: String,
    pub(crate) level: String,
    pub(crate) timestamp: String,
    #[serde(default)]
    pub(crate) stacktrace: Option<String>,
}

#[derive(Default)]
pub(crate) struct RecordingLogger {
    pub(crate) formats: Vec<DiagnosticFormat>,
    pub(crate) levels: Vec<LogLevel>,
}

impl DiagnosticLogger for RecordingLogger {
    fn has_handler(&self) -> bool {
        !self.formats.is_empty()
    }

    fn install_handler(&mut self, format: DiagnosticFormat) {
        self.formats.push(format);
    }

    fn set_level(&mut self, level: LogLevel) {
        self.levels.push(level);
    }
}
