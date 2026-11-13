use std::collections::BTreeMap;

use betterlite_context::cloud_storage::{Encoding, encode_object_name};
use betterlite_models::decisions::Json;

use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;
use crate::gcs_mock::request::{GcsRequest, GcsWriteRequest};
use crate::types::gcs_storage_auth::type_definitions::GcsLoggingPayload;

use super::bucket_folder_behaviors::handle_folders_in_bucket_name;
use super::error::GcsClientError;
use super::token_request_behaviors::to_header_map;
use super::{GcsBucketBase, RequestHeaders, STORAGE_ENDPOINT};

pub async fn log_json_data_on_gcs<Auth, Http>(
    base: &GcsBucketBase<Auth, Http>,
    headers: &RequestHeaders,
    bucket_name: &str,
    object_name: &str,
    payload: &GcsLoggingPayload,
) -> Result<Json, GcsClientError<Auth, Http>>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let json_payload = match payload {
        GcsLoggingPayload::Text(text) => text.clone(),
        GcsLoggingPayload::Standard(standard) => {
            serde_json::to_string(standard).map_err(GcsClientError::Payload)?
        }
    };
    let (bucket, object) =
        handle_folders_in_bucket_name(bucket_name, object_name).map_err(GcsClientError::Bucket)?;
    let encoded = encode_object_name(&object, Encoding::Gcs);
    let url = format!(
        "{STORAGE_ENDPOINT}/upload/storage/v1/b/{bucket}/o?uploadType=media&name={encoded}"
    );
    let request = GcsRequest::Post(GcsWriteRequest {
        url,
        data: Some(Json::Text(json_payload)),
        json: None,
        params: BTreeMap::new(),
        headers: to_header_map::<Auth, Http>(headers)?,
        timeout: None,
        stream: false,
        content: None,
    });
    let response = base
        .client
        .send(request)
        .await
        .map_err(GcsClientError::Transport)?;
    if response.status != 200 {
        tracing::error!(status = response.status, "GCS Bucket logging error");
    }
    serde_json::from_slice(&response.body).map_err(GcsClientError::Response)
}

mod tests;
