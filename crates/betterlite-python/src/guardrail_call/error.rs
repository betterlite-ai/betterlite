use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError};
use serde::Serialize;

use betterlite_guardrails::guardrail_base::error::GuardrailBaseError;
use betterlite_guardrails::guardrail_records::error::GuardrailRecordError;
use betterlite_guardrails::guardrail_selection::error::GuardrailSelectionError;
use betterlite_redact::error::RedactionError;

create_exception!(betterlite_native, NativeGuardrailError, PyException);

#[derive(Debug, thiserror::Error)]
pub(crate) enum CallError {
    #[error("the {field} could not be read as JSON of the expected shape: {source}")]
    Json {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("a value could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the violation message context value for {name} is not text")]
    ContextValue { name: String },
    #[error("{name} is not a guardrail event type")]
    EventType { name: String },
    #[error("the number {text} does not fit a 64-bit integer or an IEEE float")]
    Number { text: String },
    #[error(transparent)]
    Base {
        #[from]
        source: GuardrailBaseError,
    },
    #[error(transparent)]
    Selection {
        #[from]
        source: GuardrailSelectionError,
    },
    #[error(transparent)]
    Record {
        #[from]
        source: GuardrailRecordError,
    },
    #[error("the guardrail cache could not be built: {source}")]
    Cache {
        #[source]
        source: RedactionError,
    },
    #[error("this guardrail invocation is already finished")]
    Finished,
}

#[derive(Serialize)]
struct NativeErrorPayload<'name> {
    kind: &'static str,
    message: String,
    guardrail_name: Option<&'name str>,
}

impl CallError {
    pub(crate) const fn kind(&self) -> &'static str {
        match self {
            Self::Json { .. } | Self::Encode { .. } | Self::Number { .. } => "InvalidJson",
            Self::ContextValue { .. } => "ContextValueNotText",
            Self::EventType { .. } => "InvalidEventType",
            Self::Base { source } => match source {
                GuardrailBaseError::UnknownEventHook { .. } => "UnknownEventHook",
                GuardrailBaseError::UnsupportedEventHook { .. } => "UnsupportedEventHook",
            },
            Self::Selection { source } => match source {
                GuardrailSelectionError::TagMode { .. } => "TagMode",
                GuardrailSelectionError::RequestedNotList => "RequestedNotList",
                GuardrailSelectionError::DynamicParamsNotObject { .. } => "DynamicParamsNotObject",
            },
            Self::Record { source } => match source {
                GuardrailRecordError::Json { .. } => "Json",
                GuardrailRecordError::MessagesShape { .. } => "MessagesShape",
                GuardrailRecordError::ResponsesInput(_) => "ResponsesInput",
            },
            Self::Cache { .. } => "CacheUnavailable",
            Self::Finished => "AlreadyFinished",
        }
    }

    pub(crate) fn into_exception(self) -> PyErr {
        let kind = self.kind();
        // only a selection error carries a guardrail name, and it comes from the request rather than the message
        let guardrail_name = match &self {
            Self::Selection { source } => match source {
                GuardrailSelectionError::TagMode { guardrail_name } => guardrail_name.as_deref(),
                GuardrailSelectionError::DynamicParamsNotObject { guardrail_name } => {
                    Some(guardrail_name.as_str())
                }
                GuardrailSelectionError::RequestedNotList => None,
            },
            Self::Json { .. }
            | Self::Encode { .. }
            | Self::ContextValue { .. }
            | Self::EventType { .. }
            | Self::Number { .. }
            | Self::Base { .. }
            | Self::Record { .. }
            | Self::Cache { .. }
            | Self::Finished => None,
        };
        let payload = NativeErrorPayload {
            kind,
            message: self.to_string(),
            guardrail_name,
        };
        match serde_json::to_string(&payload) {
            Ok(text) => NativeGuardrailError::new_err(text),
            Err(_encoding) => PyRuntimeError::new_err(payload.message),
        }
    }
}

impl From<CallError> for PyErr {
    fn from(error: CallError) -> Self {
        error.into_exception()
    }
}
