use betterlite_context::cloud_storage::CloudStorageError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::{GcsHttpClient, GcsMockError};
use crate::gcs_mock::settings::GcsMockConfigError;

use super::config_resolution_behaviors::GcsConfigError;

#[derive(Debug, Error)]
pub enum GcsClientError<Auth, Http>
where
    Auth: VertexAuth,
    Http: GcsHttpClient,
{
    #[error("the GCS mock environment is invalid")]
    Mock(#[source] GcsMockConfigError),
    #[error("standard_callback_dynamic_params is not valid")]
    Dynamic(#[from] GcsConfigError),
    #[error("GCS_BUCKET_NAME is not set, but the GCS bucket is in use")]
    MissingBucketName,
    #[error("the GCS object bucket is not valid")]
    Bucket(#[source] CloudStorageError),
    #[error("a GCS request header name is not valid")]
    HeaderName(#[source] InvalidHeaderName),
    #[error("a GCS request header value is not valid")]
    Header(#[source] InvalidHeaderValue),
    #[error("the GCS request headers for bucket {bucket_name} could not be built")]
    Headers {
        bucket_name: String,
        #[source]
        source: Auth::Error,
    },
    #[error("the GCS vertex authentication for the bucket failed")]
    Token(#[source] Auth::Error),
    #[error("the GCS request failed")]
    Transport(#[source] GcsMockError<Http::Error>),
    #[error("the GCS response body is not valid JSON")]
    Response(#[source] serde_json::Error),
    #[error("the GCS logging payload could not be serialized")]
    Payload(#[source] serde_json::Error),
}
