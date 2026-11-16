use betterlite_core::api::response_chat_history::error::HistoryError;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OutputRewriteError {
    #[error("the guardrail failed the response")]
    Guardrail {
        #[source]
        source: Box<GuardrailFailure>,
    },
    #[error("the guardrail returned text {index} but only {scanned} texts were scanned")]
    TooManyTexts { index: usize, scanned: usize },
    #[error("an output tool call item could not be read")]
    ToolCallItem(#[from] HistoryError),
    #[error("the {target} could not be converted")]
    Convert {
        target: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the response field '{field}' must be a list")]
    FieldType { field: &'static str },
}
