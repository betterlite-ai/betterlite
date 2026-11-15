use std::error::Error as StdError;

use thiserror::Error;

use crate::vertex_batch_upload::line::LineError;
use crate::vertex_batch_upload::rows::error::RowError;
use crate::vertex_batch_upload::sources::error::SourceError;

#[derive(Debug, Error)]
pub enum UploadError<Mapped: StdError + 'static> {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Line(#[from] LineError),
    #[error(transparent)]
    Row(#[from] Box<RowError<Mapped>>),
}
