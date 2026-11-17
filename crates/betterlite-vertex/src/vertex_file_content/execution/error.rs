use betterlite_logging::gcs_client::gcs_storage_auth::error::GcsClientError;
use betterlite_logging::gcs_mock::auth::VertexAuth;
use betterlite_logging::gcs_mock::client::GcsHttpClient;
use thiserror::Error;

use crate::vertex_gcs_access::error::GcsAccessError;

#[derive(Debug, Error)]
pub enum FileContentError<Auth, Http>
where
    Auth: VertexAuth,
    Http: GcsHttpClient,
{
    #[error("file_id is required in file_content_request")]
    MissingFileId,
    #[error(transparent)]
    Gcs(#[from] GcsClientError<Auth, Http>),
    #[error(transparent)]
    FileId(#[from] GcsAccessError),
    #[error("Failed to download file from GCS: {file_id}")]
    Download { file_id: String },
}
