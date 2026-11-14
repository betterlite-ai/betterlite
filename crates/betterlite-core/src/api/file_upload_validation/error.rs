use crate::api::api_boundaries::ApiLimitError;
use std::io;
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
}
