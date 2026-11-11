use betterlite_models::controls::extra_body::ExtraBodyError;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct MapperError {
    pub status_code: u16,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum NonDefaultParamsError {
    #[error("{0} must be {1}")]
    WrongType(&'static str, &'static str),
    #[error(transparent)]
    ExtraBody(#[from] ExtraBodyError),
    #[error(transparent)]
    Mapper(#[from] MapperError),
}

impl NonDefaultParamsError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Mapper(error) => error.status_code,
            Self::WrongType(..) | Self::ExtraBody(_) => 400,
        }
    }
}
