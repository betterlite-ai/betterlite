use betterlite_models::controls::extra_body::ExtraBodyError;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct MapperError {
    pub status_code: u16,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum NonDefaultParamsError {
    #[error("litellm.UnsupportedParamsError: {message}")]
    Unsupported { message: String },
    #[error("{field} must be {expected}")]
    WrongType {
        field: &'static str,
        expected: &'static str,
    },
    #[error(transparent)]
    ExtraBody(#[from] ExtraBodyError),
    #[error(transparent)]
    Mapper(#[from] MapperError),
}

impl NonDefaultParamsError {
    // LiteLLM's UnsupportedParamsError ignores the status it is given and always carries 400.
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Mapper(error) => error.status_code,
            Self::Unsupported { .. } | Self::WrongType { .. } | Self::ExtraBody(_) => 400,
        }
    }
}
