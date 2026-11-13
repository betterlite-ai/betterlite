use thiserror::Error;

// LiteLLM's get_error_class picks one of its three request exception classes by the reply's status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    BadRequest,
    Authentication,
    Server,
}

impl ErrorClass {
    #[must_use]
    pub const fn status(self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::Authentication => 401,
            Self::Server => 500,
        }
    }
}

#[must_use]
pub const fn error_class(status_code: u16) -> ErrorClass {
    match status_code {
        400 => ErrorClass::BadRequest,
        401 => ErrorClass::Authentication,
        500..=u16::MAX => ErrorClass::Server,
        _ => ErrorClass::BadRequest,
    }
}

#[derive(Debug, Error)]
pub enum ModelScopeImageError {
    #[error(
        "MODELSCOPE_API_KEY is not set. Please set it via environment variable or pass api_key parameter."
    )]
    MissingApiKey,
    #[error("parameter {name} is not supported for model {model}; set drop_params to drop it")]
    UnsupportedParam { name: String, model: String },
    #[error("the ModelScope image reply is not JSON")]
    Decode {
        class: ErrorClass,
        #[source]
        source: serde_json::Error,
    },
    #[error("{message}")]
    ErrorBody { class: ErrorClass, message: String },
    #[error("the ModelScope image reply has a {field} field of the wrong type")]
    Field { field: &'static str },
}

impl ModelScopeImageError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::MissingApiKey => None,
            Self::UnsupportedParam { .. } => Some(400),
            Self::Decode { class, .. } | Self::ErrorBody { class, .. } => Some(class.status()),
            Self::Field { .. } => Some(500),
        }
    }
}
