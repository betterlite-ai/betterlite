use std::io;

use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;

#[derive(Debug, Error)]
pub enum FileUploadValidationError {
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("the uploaded file could not be read")]
    Read {
        #[source]
        source: io::Error,
    },
}
