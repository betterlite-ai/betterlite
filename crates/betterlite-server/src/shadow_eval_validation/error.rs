use betterlite_logging::private_json::error::PrivateJsonError;

#[derive(Debug, thiserror::Error)]
pub enum ShadowEvalRequestError {
    #[error(transparent)]
    Bounded(#[from] PrivateJsonError),
    #[error("the shadow evaluation request is not valid JSON")]
    Decode(#[source] serde_json::Error),
    #[error("the shadow evaluation request is not a JSON object")]
    NotAnObject,
    #[error("the shadow evaluation request field {path} is invalid: {source}")]
    Field {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("{message}")]
    Validation {
        field: &'static str,
        message: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum AccessError {
    #[error("only proxy admin roles can {action}")]
    ViewerForbidden { action: String },
    #[error("only a proxy admin can {action}")]
    WriterForbidden { action: String },
    #[error("the caller's role could not be read: {message}")]
    Caller { message: String },
}

impl AccessError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::ViewerForbidden { .. } | Self::WriterForbidden { .. } => 403,
            Self::Caller { .. } => 500,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ShadowEvalValidationError {
    #[error("Not a configured auto-router: {names}")]
    UnconfiguredRouters { names: String },
    #[error("{message}")]
    UnknownTargets { message: String },
    #[error("{field} '{model}' is an auto-router; it must be a plain model")]
    AutoRouterModel { field: &'static str, model: String },
    #[error("{message}")]
    UnreachableModel { message: String },
    #[error("{message}")]
    MissingAnthropicCredentials { message: String },
    #[error("judge_model '{judge}' is also an arm this job would judge: {collisions}")]
    JudgeIsCandidate { judge: String, collisions: String },
    #[error("models not served by this proxy: {models}")]
    UnservedModels { models: String },
}

impl ShadowEvalValidationError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::UnconfiguredRouters { .. }
            | Self::UnknownTargets { .. }
            | Self::AutoRouterModel { .. }
            | Self::UnreachableModel { .. }
            | Self::MissingAnthropicCredentials { .. }
            | Self::JudgeIsCandidate { .. }
            | Self::UnservedModels { .. } => 400,
        }
    }
}
