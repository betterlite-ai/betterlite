use std::fmt::Debug;
use thiserror::Error;

use crate::gcs_client::gcs_storage_auth::config_resolution_behaviors::logging_config;
use crate::gcs_client::gcs_storage_auth::error::GcsClientError;
use crate::gcs_client::gcs_storage_auth::header_construction_behaviors::construct_request_headers;
use crate::gcs_client::gcs_storage_auth::upload_behaviors::log_json_data_on_gcs;
use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;
use crate::types::gcs_storage_auth::type_definitions::{GcsLoggingKwargs, GcsLoggingPayload};

use super::GcsBucketLogger;
use super::QueuedItem;
use super::object_naming_behaviors::{ObjectNameError, get_object_name};

#[derive(Debug, Error)]
pub enum SingleUploadError<Auth, Http>
where
    Auth: VertexAuth + Debug,
    Http: GcsHttpClient + Debug,
{
    #[error("the GCS item upload failed")]
    Client(#[from] GcsClientError<Auth, Http>),
    #[error("the GCS object name could not be built")]
    ObjectName(#[source] ObjectNameError),
    #[error("the GCS logging payload could not be serialized")]
    Payload(#[source] serde_json::Error),
}

// LiteLLM catches every exception of a single send and logs it, so one bad item never stops the loop
pub async fn send_single_log_item<Auth, Http>(
    logger: &GcsBucketLogger<Auth, Http>,
    item: &QueuedItem,
) where
    Auth: VertexAuth + Clone + Debug,
    Http: GcsHttpClient + Debug,
{
    if let Err(error) = upload_item(logger, item).await {
        tracing::error!(
            error = ?error,
            "GCS Bucket error logging individual payload to GCS bucket"
        );
    }
}

async fn upload_item<Auth, Http>(
    logger: &GcsBucketLogger<Auth, Http>,
    item: &QueuedItem,
) -> Result<(), SingleUploadError<Auth, Http>>
where
    Auth: VertexAuth + Clone + Debug,
    Http: GcsHttpClient + Debug,
{
    let kwargs = GcsLoggingKwargs {
        dynamic_params: Some(item.dynamic_params.as_ref().clone()),
    };
    let config = logging_config(&logger.base, &kwargs).await?;
    let headers = construct_request_headers(
        &logger.base,
        &config.bucket_name,
        config.path_service_account.as_ref(),
        Some(&config.vertex_instance),
    )
    .await?;
    let object_name = get_object_name(
        logger,
        item.payload.as_ref(),
        &item.response_id,
        item.metadata.as_ref(),
    )
    .map_err(SingleUploadError::ObjectName)?;
    let payload =
        serde_json::to_string(item.payload.as_ref()).map_err(SingleUploadError::Payload)?;
    log_json_data_on_gcs(
        &logger.base,
        &headers,
        &config.bucket_name,
        &object_name,
        &GcsLoggingPayload::Text(payload),
    )
    .await?;
    Ok(())
}

mod tests;
