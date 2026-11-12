use crate::non_default_params::error::{MapperError, NonDefaultParamsError};

#[derive(Debug, thiserror::Error)]
pub enum EmbeddingParamsError {
    #[error("{provider} does not support the passed parameters for {model}")]
    Unsupported { provider: String, model: String },
    #[error("dimensions is not supported for {model}")]
    DimensionsUnsupported { model: String },
    #[error("{0} must be {1}")]
    WrongType(&'static str, &'static str),
    #[error("the {provider} embedding branch needs its provider config")]
    MissingMapper { provider: String },
    #[error(transparent)]
    Params(#[from] NonDefaultParamsError),
    #[error(transparent)]
    Mapper(#[from] MapperError),
}

impl EmbeddingParamsError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Params(error) => error.status_code(),
            Self::Mapper(error) => error.status_code,
            Self::MissingMapper { .. } => 500,
            Self::Unsupported { .. } | Self::DimensionsUnsupported { .. } | Self::WrongType(..) => {
                400
            }
        }
    }
}
