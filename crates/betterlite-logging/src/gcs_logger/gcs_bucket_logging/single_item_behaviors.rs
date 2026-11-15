use crate::gcs_client::gcs_storage_auth::config_resolution_behaviors::logging_config;
use crate::gcs_client::gcs_storage_auth::header_construction_behaviors::construct_request_headers;
use crate::gcs_client::gcs_storage_auth::upload_behaviors::log_json_data_on_gcs;
use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;
use crate::types::gcs_storage_auth::type_definitions::{GcsLoggingKwargs, GcsLoggingPayload};

use super::GcsBucketLogger;
use super::QueuedItem;
use super::grouped_batch_behaviors::dynamic_params;
use super::object_naming_behaviors::get_object_name;

// LiteLLM catches every exception of a single send and logs it, so one bad item never stops the loop
pub async fn send_single_log_item<Auth, Http>(
    logger: &GcsBucketLogger<Auth, Http>,
    item: &QueuedItem,
) where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    if let Err(message) = upload_item(logger, item).await {
        tracing::error!(
            message = %message,
            "GCS Bucket error logging individual payload to GCS bucket"
        );
    }
}

async fn upload_item<Auth, Http>(
    logger: &GcsBucketLogger<Auth, Http>,
    item: &QueuedItem,
) -> Result<(), String>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let kwargs = GcsLoggingKwargs {
        dynamic_params: dynamic_params(item.dynamic_params.as_ref()),
    };
    let config = logging_config(&logger.base, &kwargs)
        .await
        .map_err(|error| error.to_string())?;
    let headers = construct_request_headers(
        &logger.base,
        &config.bucket_name,
        config.path_service_account.as_ref(),
        Some(&config.vertex_instance),
    )
    .await
    .map_err(|error| error.to_string())?;
    let object_name = get_object_name(
        logger,
        item.payload.as_ref(),
        &item.response_id,
        item.metadata.as_ref(),
    )
    .map_err(|error| error.to_string())?;
    let payload =
        serde_json::to_string(item.payload.as_ref()).map_err(|error| error.to_string())?;
    log_json_data_on_gcs(
        &logger.base,
        &headers,
        &config.bucket_name,
        &object_name,
        &GcsLoggingPayload::Text(payload),
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

mod tests;
