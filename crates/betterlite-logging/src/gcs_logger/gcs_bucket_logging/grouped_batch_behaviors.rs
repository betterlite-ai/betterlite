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
use super::naming_format_behaviors::{generate_batch_object_name, get_object_date};
use super::ndjson_behaviors::combine_payloads_to_ndjson;

#[derive(Debug, Error)]
pub enum BatchUploadError<Auth, Http>
where
    Auth: VertexAuth + Debug,
    Http: GcsHttpClient + Debug,
{
    #[error("the GCS batch upload failed")]
    Client(#[from] GcsClientError<Auth, Http>),
    #[error("the GCS batch payload could not be serialized")]
    Payload(#[source] serde_json::Error),
}

#[must_use]
pub async fn send_grouped_batch<Auth, Http>(
    logger: &GcsBucketLogger<Auth, Http>,
    items: &[QueuedItem],
) -> (usize, usize)
where
    Auth: VertexAuth + Clone + Debug,
    Http: GcsHttpClient + Debug,
{
    if items.is_empty() {
        return (0, 0);
    }
    match upload_group(logger, items).await {
        Ok(()) => (items.len(), 0),
        Err(error) => {
            tracing::error!(
                error = ?error,
                "GCS Bucket error logging batch payload to GCS bucket"
            );
            (0, items.len())
        }
    }
}

async fn upload_group<Auth, Http>(
    logger: &GcsBucketLogger<Auth, Http>,
    items: &[QueuedItem],
) -> Result<(), BatchUploadError<Auth, Http>>
where
    Auth: VertexAuth + Clone + Debug,
    Http: GcsHttpClient + Debug,
{
    let kwargs = logging_kwargs(items);
    let config = logging_config(&logger.base, &kwargs).await?;
    let headers = construct_request_headers(
        &logger.base,
        &config.bucket_name,
        config.path_service_account.as_ref(),
        Some(&config.vertex_instance),
    )
    .await?;
    let date = get_object_date(logger.clock.now());
    let batch_id = batch_id(logger);
    let object_name = generate_batch_object_name(&date, &batch_id);
    let ndjson = combine_payloads_to_ndjson(items).map_err(BatchUploadError::Payload)?;
    log_json_data_on_gcs(
        &logger.base,
        &headers,
        &config.bucket_name,
        &object_name,
        &GcsLoggingPayload::Text(ndjson),
    )
    .await?;
    Ok(())
}

fn logging_kwargs(items: &[QueuedItem]) -> GcsLoggingKwargs {
    let Some(first) = items.first() else {
        return GcsLoggingKwargs::default();
    };
    GcsLoggingKwargs {
        dynamic_params: Some(first.dynamic_params.as_ref().clone()),
    }
}

// the first item's kwargs pick the bucket and the credentials for the whole group
fn batch_id<Auth, Http>(logger: &GcsBucketLogger<Auth, Http>) -> String {
    let millis = logger.clock.now().timestamp_millis();
    let uuid = logger.ids.uuid_hex();
    let suffix = uuid.get(..8).unwrap_or_default();
    format!("{millis}-{suffix}")
}

mod tests;
