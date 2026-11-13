use std::collections::BTreeMap;
use std::time::TryFromFloatSecsError;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use thiserror::Error;

use super::runtime::LoadFailure;
use super::runtime::RuntimeFailure;
use crate::custom_code_execution::execution::error::ExecutionError;
use crate::custom_code_execution::execution::error::SandboxError;
use crate::custom_code_execution::sandbox::error::CompileError;
use crate::generic_guardrail_contracts::GuardrailInputType;

pub type ErrorDetails = BTreeMap<String, GuardrailValue>;

#[derive(Debug, Error)]
pub enum CustomCodeGuardrailError {
    #[error("execution_timeout must be positive, got {timeout}")]
    NonPositiveTimeout { timeout: f64 },
    #[error("the execution timeout {timeout} is out of range")]
    TimeoutOutOfRange {
        timeout: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("Custom code guardrail requires a guardrail_name")]
    MissingGuardrailName,
    #[error("Custom code guardrail requires 'custom_code' in litellm_params")]
    MissingCustomCode,
    #[error(
        "Custom code must define an 'apply_guardrail' function. Expected signature: apply_guardrail(inputs, request_data, input_type)"
    )]
    MissingFunction,
    #[error("'apply_guardrail' must be a callable function")]
    NotCallable,
    #[error("Syntax error in custom code: {source}")]
    Syntax {
        #[source]
        source: CompileError,
    },
    #[error("Failed to compile custom code: {source}")]
    CompileFailed {
        #[source]
        source: ExecutionError<LoadFailure>,
    },
    #[error("Custom code guardrail execution failed: {source}")]
    Execution {
        #[source]
        source: ExecutionError<RuntimeFailure>,
        details: ErrorDetails,
    },
    #[error("Custom code guardrail '{guardrail_name}' exceeded its {timeout}s execution timeout")]
    Timeout {
        guardrail_name: String,
        timeout: f64,
        details: ErrorDetails,
    },
    #[error("{message}")]
    ExecutionMessage { message: String },
}

impl CustomCodeGuardrailError {
    #[must_use]
    pub const fn details(&self) -> Option<&ErrorDetails> {
        match self {
            Self::Execution { details, .. } | Self::Timeout { details, .. } => Some(details),
            Self::NonPositiveTimeout { .. }
            | Self::TimeoutOutOfRange { .. }
            | Self::MissingGuardrailName
            | Self::MissingCustomCode
            | Self::MissingFunction
            | Self::NotCallable
            | Self::Syntax { .. }
            | Self::CompileFailed { .. }
            | Self::ExecutionMessage { .. } => None,
        }
    }
}

#[derive(Debug, Error)]
pub enum CustomCodeApplyError {
    #[error("{reason}")]
    Blocked {
        reason: String,
        guardrail: String,
        detection_info: ErrorDetails,
    },
    #[error("{message}")]
    Passthrough {
        message: String,
        detection_info: ErrorDetails,
    },
    #[error(transparent)]
    Execution(#[from] CustomCodeGuardrailError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDetail<'data> {
    pub error: &'data str,
    pub guardrail: &'data str,
    pub detection_info: &'data ErrorDetails,
}

impl CustomCodeApplyError {
    // A block is HTTP 400 after the model call and a passthrough before it; execution errors are 500.
    #[must_use]
    pub const fn status_code(&self) -> Option<StatusCode> {
        match self {
            Self::Blocked { .. } => Some(StatusCode::BAD_REQUEST),
            Self::Passthrough { .. } => None,
            Self::Execution(_) => Some(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    #[must_use]
    pub fn block_detail(&self) -> Option<BlockDetail<'_>> {
        match self {
            Self::Blocked {
                reason,
                guardrail,
                detection_info,
            } => Some(BlockDetail {
                error: reason,
                guardrail,
                detection_info,
            }),
            Self::Passthrough { .. } | Self::Execution(_) => None,
        }
    }
}

#[must_use]
pub fn load_error(source: ExecutionError<LoadFailure>) -> CustomCodeGuardrailError {
    match source {
        ExecutionError::Sandbox(SandboxError::Failed(LoadFailure::MissingFunction)) => {
            CustomCodeGuardrailError::MissingFunction
        }
        ExecutionError::Sandbox(SandboxError::Failed(LoadFailure::NotCallable)) => {
            CustomCodeGuardrailError::NotCallable
        }
        other @ (ExecutionError::Timeout(_)
        | ExecutionError::Sandbox(_)
        | ExecutionError::Spawn { .. }
        | ExecutionError::TimeoutOutOfRange { .. }) => {
            CustomCodeGuardrailError::CompileFailed { source: other }
        }
    }
}

#[must_use]
pub fn execution_error(
    guardrail_name: &str,
    execution_timeout: f64,
    input_type: GuardrailInputType,
    source: ExecutionError<RuntimeFailure>,
) -> CustomCodeGuardrailError {
    match source {
        ExecutionError::Timeout(_) | ExecutionError::Sandbox(SandboxError::Interrupted(_)) => {
            timeout_failure(guardrail_name, execution_timeout, input_type)
        }
        other @ (ExecutionError::Sandbox(_)
        | ExecutionError::Spawn { .. }
        | ExecutionError::TimeoutOutOfRange { .. }) => {
            tracing::error!("Custom code guardrail '{guardrail_name}' execution error: {other}");
            CustomCodeGuardrailError::Execution {
                source: other,
                details: execution_details(guardrail_name, input_type),
            }
        }
    }
}

fn timeout_failure(
    guardrail_name: &str,
    execution_timeout: f64,
    input_type: GuardrailInputType,
) -> CustomCodeGuardrailError {
    tracing::error!(
        "Custom code guardrail '{guardrail_name}' exceeded its {execution_timeout}s execution timeout"
    );
    let mut details = execution_details(guardrail_name, input_type);
    details.insert(
        "timeout".to_owned(),
        GuardrailValue::Decimal(execution_timeout),
    );
    CustomCodeGuardrailError::Timeout {
        guardrail_name: guardrail_name.to_owned(),
        timeout: execution_timeout,
        details,
    }
}

fn execution_details(guardrail_name: &str, input_type: GuardrailInputType) -> ErrorDetails {
    let mut details = ErrorDetails::new();
    details.insert(
        "guardrail_name".to_owned(),
        GuardrailValue::Text(guardrail_name.to_owned()),
    );
    details.insert(
        "input_type".to_owned(),
        GuardrailValue::Text(super::runtime::input_type_name(input_type).to_owned()),
    );
    details
}
