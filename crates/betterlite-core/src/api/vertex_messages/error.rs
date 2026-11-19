use std::error::Error as StdError;

use thiserror::Error;

use crate::api::messages_native_transform::error::TransformError;

#[derive(Debug, Error)]
pub enum NativeVertexMessagesError<OverrideError>
where
    OverrideError: StdError + 'static,
{
    #[error(transparent)]
    Transform(#[from] TransformError),
    #[error("the Vertex native Messages override failed: {0}")]
    Overrides(#[source] OverrideError),
}

impl<OverrideError> NativeVertexMessagesError<OverrideError>
where
    OverrideError: StdError + 'static,
{
    // The parent transform owns the 400s; a failing override is an internal error.
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Transform(error) => error.status_code(),
            Self::Overrides(_) => 500,
        }
    }
}
