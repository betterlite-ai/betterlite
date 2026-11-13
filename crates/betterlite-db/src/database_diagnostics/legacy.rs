use betterlite_redact::secret::Redactor;

use crate::database_diagnostics::message::format_message;
use crate::database_diagnostics::record::{FormatError, LogRecord, render_exception};
use crate::database_diagnostics::timestamp::{LocalTime, legacy_time};

pub struct TextFormatter<'dependencies> {
    pub local_time: &'dependencies dyn LocalTime,
    pub redactor: &'dependencies Redactor,
}

impl TextFormatter<'_> {
    pub fn format(&self, record: &LogRecord<'_>) -> Result<String, FormatError> {
        let message = format_message(record.message, &record.arguments)
            .map_err(|source| FormatError::Message { source })?;
        let timestamp = legacy_time(self.local_time, record.created)
            .map_err(|source| FormatError::Timestamp { source })?;
        let level = record.level.to_uppercase();
        let mut rendered = format!("{timestamp} - litellm_proxy_extras - {level} - {message}");
        if let Some(exception) = record.exception {
            rendered.push('\n');
            rendered.push_str(&render_exception(exception, self.redactor));
        }
        Ok(rendered)
    }
}
