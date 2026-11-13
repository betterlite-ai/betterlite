use crate::database_diagnostics::message::LogValue;

#[derive(Debug, Clone, PartialEq)]
pub enum GlobalJsonLogs {
    Missing,
    Supplied(LogValue),
    ImportError,
    AttributeError,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JsonLogsError {
    #[error("the litellm json_logs attribute could not be read")]
    Lookup,
}

pub fn is_json_enabled(
    global: &GlobalJsonLogs,
    environment: Option<&str>,
) -> Result<bool, JsonLogsError> {
    match global {
        GlobalJsonLogs::Missing => Ok(false),
        GlobalJsonLogs::Supplied(value) => Ok(value.is_enabled()),
        GlobalJsonLogs::ImportError | GlobalJsonLogs::AttributeError => {
            Ok(environment_enabled(environment))
        }
        GlobalJsonLogs::Failed => Err(JsonLogsError::Lookup),
    }
}

#[must_use]
pub fn environment_enabled(environment: Option<&str>) -> bool {
    match environment {
        Some(text) => text.to_lowercase() == "true",
        None => false,
    }
}
