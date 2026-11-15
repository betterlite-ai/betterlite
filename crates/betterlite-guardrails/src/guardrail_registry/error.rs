use betterlite_config::sections::guardrail_params::error::GuardrailParamsError;
use thiserror::Error;

use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_initializers::error::InitializerError;

use super::rows::RowsError;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("Unsupported guardrail: {kind}")]
    Unsupported { kind: String },
    #[error("guardrail_type is required")]
    MissingGuardrailType,
    #[error(
        "Guardrail {guardrail_name}: scan_only_tool_results is enabled, but this guardrail's role filtering never scans tool results, so no request content would ever be scanned. Remove scan_only_tool_results or the guardrail's role-filtering option."
    )]
    ScanOnlyUnsupported { guardrail_name: String },
    #[error(
        "Guardrail {guardrail_name}: scan_only_tool_results and skip_tool_message_in_guardrail are enabled together, which excludes every message from scanning, so no request content would ever be scanned. Remove one of the two."
    )]
    ScanOnlyWithSkipTool { guardrail_name: String },
    #[error(transparent)]
    Base(#[from] GuardrailBaseError),
    // LiteLLM raises ValueError for a bad guardrail config, so a route answers 400 and startup skips it
    #[error(transparent)]
    Initializer(#[from] InitializerError),
    // LiteLLM raises a plain Exception for the missing enterprise package, so a route still answers 200
    #[error(transparent)]
    Enterprise(InitializerError),
    #[error(transparent)]
    Rows(#[from] RowsError),
    #[error(transparent)]
    Params(#[from] GuardrailParamsError),
    #[error("stored guardrail row is not valid JSON")]
    Row {
        #[source]
        source: serde_json::Error,
    },
    #[error("guardrail parameters could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("guardrail parameters could not be decoded")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("stored guardrail row has no litellm_params")]
    MissingParams,
    #[error(
        "Error updating guardrail from DB: Guardrail not found, passed guardrail_id={guardrail_id}"
    )]
    UpdateNotFound { guardrail_id: String },
    #[error("{context}: {source}")]
    Context {
        context: &'static str,
        #[source]
        source: Box<Self>,
    },
    #[error("Guardrail initialization failed: {source}")]
    InitFailed {
        #[source]
        source: Box<Self>,
    },
}

// LiteLLM wraps every failure of a DB method in its own text, the not-found update included
pub(crate) fn wrapped(context: &'static str, source: RegistryError) -> RegistryError {
    RegistryError::Context {
        context,
        source: Box::new(source),
    }
}
