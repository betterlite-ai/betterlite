use std::error::Error as StdError;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use thiserror::Error;

use crate::guardrail_metadata::GuardrailMetadataError;
use crate::guardrail_records::error::GuardrailRecordError;
use crate::guardrail_selection::error::GuardrailSelectionError;
use crate::model_armor::decisions::ModelArmorAPIError;
use crate::model_armor::decisions::model_response::ResponseContentError;
use crate::model_armor::decisions::raise_if_fail_closed;
use crate::model_armor::request_files::ScanError;
use crate::model_armor::transport::ArmorResponse;
use crate::model_armor::transport::error::{ArmorCallError, TransportError};

#[derive(Debug, Error)]
#[error("the request field {field} must be {expected}")]
pub struct FieldError {
    pub field: &'static str,
    pub expected: &'static str,
    #[source]
    pub source: Option<serde_json::Error>,
}

#[derive(Debug, Error)]
pub enum HookError<E: StdError + Send + Sync + 'static> {
    #[error("Model Armor blocked the request")]
    Blocked {
        status: u16,
        detail: IndexMap<String, GuardrailValue>,
    },
    #[error(transparent)]
    Api(#[from] ModelArmorAPIError),
    #[error(transparent)]
    Transport(#[from] TransportError<E>),
    #[error(transparent)]
    Selection(#[from] GuardrailSelectionError),
    #[error(transparent)]
    Field(#[from] FieldError),
    #[error(transparent)]
    Response(#[from] ResponseContentError),
    #[error(transparent)]
    Metadata(#[from] GuardrailMetadataError),
    #[error("the Model Armor guardrail record could not be stored")]
    Record {
        #[source]
        source: GuardrailRecordError,
    },
    #[error("the request metadata is not a JSON value")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}

impl<E: StdError + Send + Sync + 'static> From<ScanError<E>> for HookError<E> {
    fn from(error: ScanError<E>) -> Self {
        match error {
            ScanError::Blocked { status, detail } => Self::Blocked { status, detail },
            ScanError::Api(source) => Self::Api(source),
            ScanError::Transport(source) => Self::Transport(source),
        }
    }
}

// LiteLLM's error clauses: a Model Armor API error obeys fail_on_error, any other error is logged first
pub(super) fn call_outcome<E>(
    result: Result<ArmorResponse, ArmorCallError<E>>,
    fail_on_error: bool,
    hook: &'static str,
) -> Result<Option<ArmorResponse>, HookError<E>>
where
    E: StdError + Send + Sync + 'static,
{
    match result {
        Ok(response) => Ok(Some(response)),
        Err(ArmorCallError::Api(error)) => {
            raise_if_fail_closed(error, fail_on_error)?;
            Ok(None)
        }
        Err(ArmorCallError::Transport(error)) => {
            tracing::error!(error = %error, hook, "Model Armor request failed");
            if fail_on_error {
                return Err(HookError::Transport(error));
            }
            Ok(None)
        }
    }
}

// LiteLLM's `except HTTPException: raise`, then `except Exception` with the fail_on_error check
pub(super) fn hook_outcome<E>(
    result: Result<(), HookError<E>>,
    fail_on_error: bool,
    hook: &'static str,
) -> Result<(), HookError<E>>
where
    E: StdError + Send + Sync + 'static,
{
    let error = match result {
        Ok(()) => return Ok(()),
        Err(error @ (HookError::Blocked { .. } | HookError::Selection(_))) => return Err(error),
        Err(error) => error,
    };
    tracing::error!(error = %error, hook, "Model Armor error");
    if fail_on_error { Err(error) } else { Ok(()) }
}

mod tests;
