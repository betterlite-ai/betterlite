use std::str::Utf8Error;

use betterlite_context::cloud_storage::CloudStorageError;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GcsAccessError {
    #[error("GCS bucket_name is required")]
    BucketRequired,
    #[error("file identifier exceeds the {limit}-byte limit")]
    FileIdTooLong { limit: usize },
    #[error("file identifier is not UTF-8")]
    FileIdNotUtf8(#[source] Utf8Error),
    #[error(transparent)]
    Storage(#[from] CloudStorageError),
}
