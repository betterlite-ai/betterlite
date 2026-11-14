use crate::api::api_boundaries::ApiLimitError;
use std::io;
use std::num::TryFromIntError;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum FileUploadValidationError {
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("the uploaded file could not be read")]
    Read {
        #[from]
        source: io::Error,
    },
    #[error("the uploaded file size does not fit a u128")]
    Size(#[from] TryFromIntError),
}
