use betterlite_redact::secret::Redactor;
use serde::Serialize;

use crate::database_diagnostics::message::format_message;
use crate::database_diagnostics::record::{FormatError, LogRecord, render_exception};
use crate::database_diagnostics::timestamp::{LocalTime, format_time};

pub struct JsonFormatter<'dependencies> {
    pub local_time: &'dependencies dyn LocalTime,
    pub redactor: &'dependencies Redactor,
}

#[derive(Serialize)]
struct JsonRecord<'record> {
    message: &'record str,
    level: &'record str,
    timestamp: &'record str,
    #[serde(skip_serializing_if = "Option::is_none")]
    stacktrace: Option<&'record str>,
}

impl JsonFormatter<'_> {
    pub fn format(&self, record: &LogRecord<'_>) -> Result<String, FormatError> {
        let message = format_message(record.message, &record.arguments)
            .map_err(|source| FormatError::Message { source })?;
        let timestamp = format_time(self.local_time, record.created, None)
            .map_err(|source| FormatError::Timestamp { source })?;
        let level = record.level.to_uppercase();
        let stacktrace = record
            .exception
            .map(|exception| render_exception(exception, self.redactor));
        serde_json::to_string(&JsonRecord {
            message: &message,
            level: &level,
            timestamp: &timestamp,
            stacktrace: stacktrace.as_deref(),
        })
        .map_err(|source| FormatError::Serialize { source })
    }
}
