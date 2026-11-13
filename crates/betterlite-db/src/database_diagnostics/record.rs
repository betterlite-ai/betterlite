use betterlite_redact::secret::Redactor;

use crate::database_diagnostics::message::{LogArguments, MessageError};
use crate::database_diagnostics::timestamp::TimestampError;

pub struct LogRecord<'record> {
    pub created: f64,
    pub level: &'record str,
    pub message: &'record str,
    pub arguments: LogArguments,
    pub exception: Option<ExceptionRecord<'record>>,
}

#[derive(Debug, Clone, Copy)]
pub struct ExceptionRecord<'record> {
    pub kind: &'record str,
    pub message: &'record str,
}

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("the log message could not be formatted")]
    Message {
        #[source]
        source: MessageError,
    },
    #[error("the log record's timestamp could not be formatted")]
    Timestamp {
        #[source]
        source: TimestampError,
    },
    #[error("the log record could not be serialized as JSON")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
}

// the native trace is the typed cause with its values redacted, never arbitrary Python traceback text
#[must_use]
pub fn render_exception(exception: ExceptionRecord<'_>, redactor: &Redactor) -> String {
    let mut rendered = String::from(exception.kind);
    rendered.push_str(": ");
    rendered.push_str(&redactor.redact_internal_details(exception.message));
    rendered
}
