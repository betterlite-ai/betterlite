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
    #[error("{message}")]
    Compilation {
        message: String,
        details: ErrorDetails,
    },
    #[error("{message}")]
    Execution {
        message: String,
        details: ErrorDetails,
    },
}

impl CustomCodeGuardrailError {
    #[must_use]
    pub const fn details(&self) -> Option<&ErrorDetails> {
        match self {
            Self::Compilation { details, .. } | Self::Execution { details, .. } => Some(details),
            Self::NonPositiveTimeout { .. }
            | Self::TimeoutOutOfRange { .. }
            | Self::MissingGuardrailName
            | Self::MissingCustomCode => None,
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
pub fn syntax_error(source: &CompileError) -> CustomCodeGuardrailError {
    CustomCodeGuardrailError::Compilation {
        message: format!("Syntax error in custom code: {source}"),
        details: ErrorDetails::new(),
    }
}

#[must_use]
pub fn load_error(source: ExecutionError<LoadFailure>) -> CustomCodeGuardrailError {
    match source {
        ExecutionError::Sandbox(SandboxError::Failed(
            failure @ (LoadFailure::MissingFunction | LoadFailure::NotCallable),
        )) => CustomCodeGuardrailError::Compilation {
            message: failure.to_string(),
            details: ErrorDetails::new(),
        },
        other @ (ExecutionError::Timeout(_)
        | ExecutionError::Sandbox(_)
        | ExecutionError::Spawn { .. }
        | ExecutionError::TimeoutOutOfRange { .. }) => CustomCodeGuardrailError::Compilation {
            message: format!("Failed to compile custom code: {other}"),
            details: ErrorDetails::new(),
        },
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
                message: format!("Custom code guardrail execution failed: {other}"),
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
    CustomCodeGuardrailError::Execution {
        message: format!(
            "Custom code guardrail '{guardrail_name}' exceeded its {execution_timeout}s execution timeout"
        ),
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
