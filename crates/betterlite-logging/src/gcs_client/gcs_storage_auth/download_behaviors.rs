use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_context::cloud_storage::{Encoding, encode_object_name};
use http::HeaderMap;

use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;
use crate::gcs_mock::request::{GcsReadRequest, GcsRequest};
use crate::types::gcs_storage_auth::type_definitions::{GcsLoggingConfig, GcsLoggingKwargs};

use super::bucket_folder_behaviors::handle_folders_in_bucket_name;
use super::config_resolution_behaviors::logging_config;
use super::error::GcsClientError;
use super::header_construction_behaviors::construct_request_headers;
use super::token_request_behaviors::to_header_map;
use super::{GcsBucketBase, STORAGE_ENDPOINT};

pub async fn download_gcs_object<Auth, Http>(
    base: &GcsBucketBase<Auth, Http>,
    object_name: &str,
    kwargs: &GcsLoggingKwargs,
) -> Option<Vec<u8>>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let config = match logging_config(base, kwargs).await {
        Ok(config) => config,
        Err(error) => return failed_download(&error),
    };
    let headers = match download_headers(base, &config).await {
        Ok(headers) => headers,
        Err(error) => return failed_download(&error),
    };
    let (bucket, object) = match handle_folders_in_bucket_name(&config.bucket_name, object_name) {
        Ok(names) => names,
        Err(error) => return failed_download(&error),
    };
    let request = read_request(&bucket, &object, headers);
    match base.client.send(request).await {
        Err(error) => failed_download(&error),
        Ok(response) if response.status == 200 => Some(response.body),
        Ok(response) => {
            tracing::error!(status = response.status, "GCS object download error");
            None
        }
    }
}

async fn download_headers<Auth, Http>(
    base: &GcsBucketBase<Auth, Http>,
    config: &GcsLoggingConfig<Auth>,
) -> Result<HeaderMap, GcsClientError<Auth, Http>>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let constructed = construct_request_headers(
        base,
        &config.bucket_name,
        config.path_service_account.as_ref(),
        Some(&config.vertex_instance),
    )
    .await?;
    let map = to_header_map::<Auth, Http>(&constructed)?;
    Ok(map)
}

fn read_request(bucket: &str, object: &str, headers: HeaderMap) -> GcsRequest {
    let encoded = encode_object_name(object, Encoding::Gcs);
    let url = format!("{STORAGE_ENDPOINT}/storage/v1/b/{bucket}/o/{encoded}?alt=media");
    GcsRequest::Get(GcsReadRequest {
        url,
        params: BTreeMap::new(),
        headers,
        follow_redirects: None,
    })
}

fn failed_download(error: &dyn Display) -> Option<Vec<u8>> {
    tracing::error!(%error, "GCS object download error");
    None
}

mod tests;
