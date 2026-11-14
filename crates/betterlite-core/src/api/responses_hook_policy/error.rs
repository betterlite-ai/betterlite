use betterlite_logging::analytics::python::FormatError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgenticLoopSettingsError {
    #[error("litellm_params.max_agentic_loops must be an integer, got {literal}")]
    CeilingNotInteger { literal: String },
    #[error("litellm_params.max_agentic_loops must be at least 1, got {value}")]
    CeilingBelowMinimum { value: i64 },
    #[error("invalid literal for int() with base 10: {literal}")]
    DepthInvalidLiteral { literal: String },
    #[error(
        "int() argument must be a string, a bytes-like object or a real number, not '{type_name}'"
    )]
    DepthWrongType { type_name: &'static str },
    #[error("cannot convert float infinity to integer")]
    DepthInfinity,
    #[error("cannot convert float NaN to integer")]
    DepthNan,
    #[error("_agentic_loop_depth is outside the supported 64-bit range")]
    DepthOutOfRange,
    #[error("'{type_name}' object is not iterable")]
    FingerprintsNotIterable { type_name: &'static str },
    #[error("the rejected value could not be rendered as Python text")]
    Render {
        #[from]
        source: FormatError,
    },
}

impl AgenticLoopSettingsError {
    #[must_use]
    pub const fn python_class(&self) -> &'static str {
        match self {
            Self::CeilingNotInteger { .. }
            | Self::DepthWrongType { .. }
            | Self::FingerprintsNotIterable { .. }
            | Self::Render { .. } => "TypeError",
            Self::CeilingBelowMinimum { .. }
            | Self::DepthInvalidLiteral { .. }
            | Self::DepthNan
            | Self::DepthOutOfRange => "ValueError",
            Self::DepthInfinity => "OverflowError",
        }
    }
}

// AgenticLoopSafetyError subclasses ValueError in litellm/types/integrations/custom_logger.py
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgenticLoopSafetyError {
    #[error("Agentic loop detected repeated tool-call fingerprint; aborting rerun")]
    RepeatedFingerprint,
    #[error("Exceeded max_agentic_loops={max_loops} for model={model}")]
    MaxLoopsExceeded { max_loops: u32, model: String },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgenticFollowupKwargsError {
    #[error("the agentic loop depth is too large to advance")]
    DepthOverflow,
}

#[derive(Debug, Error)]
pub enum DeploymentHookError {
    #[error("the deployment hook failed: {source}")]
    Failed {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("the hook result could not be rendered as Python text: {source}")]
    Render {
        #[source]
        source: FormatError,
    },
}
