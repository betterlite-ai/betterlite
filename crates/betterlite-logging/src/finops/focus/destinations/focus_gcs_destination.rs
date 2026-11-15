use std::collections::BTreeMap;
use std::fmt::Debug;
use std::str::from_utf8;

use betterlite_context::cloud_storage::{Encoding, encode_object_name};
use http::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::focus_records::destinations::{FocusDestination, FocusTimeWindow};
use crate::gcs_client::gcs_storage_auth::error::GcsClientError;
use crate::gcs_client::gcs_storage_auth::header_construction_behaviors::construct_request_headers;
use crate::gcs_client::gcs_storage_auth::{CONTENT_TYPE_HEADER, GcsBucketBase};
use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::{GcsHttpClient, GcsMockError};
use crate::gcs_mock::request::{GcsRequest, GcsWriteRequest};

use super::object_key;

pub const MEDIA_UPLOAD_ROOT: &str = "https://storage.googleapis.com/upload/storage/v1/b";
pub const MEDIA_CONTENT_TYPE: &str = "application/octet-stream";

#[derive(Debug, Default)]
pub struct GcsSettings {
    pub bucket_name: Option<String>,
    pub service_account_json: Option<SecretString>,
}

#[derive(Debug, thiserror::Error)]
pub enum GcsSettingsError {
    #[error("bucket_name must be provided for the GCS FOCUS destination")]
    MissingBucketName,
}

#[derive(Debug, thiserror::Error)]
pub enum GcsDeliveryError<Auth, Http>
where
    Auth: VertexAuth + Debug + 'static,
    Http: GcsHttpClient + Debug + 'static,
{
    #[error("the GCS request headers could not be constructed")]
    Headers(#[source] GcsClientError<Auth, Http>),
    #[error("a GCS request header name is not valid")]
    HeaderName(#[source] InvalidHeaderName),
    #[error("a GCS request header value is not valid")]
    HeaderValue(#[source] InvalidHeaderValue),
    #[error("the GCS media upload could not be sent")]
    Transport(#[source] GcsMockError<Http::Error>),
    #[error("the GCS media upload was refused with status {status}: {body}")]
    Status { status: u16, body: String },
}

pub struct FocusGcsDestination<Auth, Http> {
    pub(super) base: GcsBucketBase<Auth, Http>,
    pub(super) bucket_name: String,
    pub(super) prefix: String,
    pub(super) service_account_json: Option<SecretString>,
}

impl<Auth, Http> FocusGcsDestination<Auth, Http> {
    pub fn new(
        base: GcsBucketBase<Auth, Http>,
        prefix: &str,
        settings: GcsSettings,
    ) -> Result<Self, GcsSettingsError> {
        let Some(bucket_name) = settings.bucket_name.filter(|name| !name.is_empty()) else {
            return Err(GcsSettingsError::MissingBucketName);
        };
        // LiteLLM keeps the GCS_PATH_SERVICE_ACCOUNT default the base read unless the config names a path
        let service_account_json = settings
            .service_account_json
            .or_else(|| base.path_service_account.clone());
        Ok(Self {
            base,
            bucket_name,
            prefix: prefix.trim_end_matches('/').to_owned(),
            service_account_json,
        })
    }
}

impl<Auth, Http> FocusGcsDestination<Auth, Http>
where
    Auth: VertexAuth + Clone + Debug + 'static,
    Http: GcsHttpClient + Debug + 'static,
{
    async fn request_headers(&self) -> Result<HeaderMap, GcsDeliveryError<Auth, Http>> {
        let mut auth_headers = construct_request_headers(
            &self.base,
            &self.bucket_name,
            self.service_account_json.as_ref(),
            None,
        )
        .await
        .map_err(GcsDeliveryError::Headers)?;
        // LiteLLM replaces the application/json type the GCS base sets with the media type
        auth_headers.insert(
            CONTENT_TYPE_HEADER.to_owned(),
            SecretString::from(MEDIA_CONTENT_TYPE),
        );
        let mut headers = HeaderMap::new();
        for (name, value) in &auth_headers {
            let mut header_value = HeaderValue::from_str(value.expose_secret())
                .map_err(GcsDeliveryError::HeaderValue)?;
            header_value.set_sensitive(true);
            headers.insert(
                HeaderName::from_bytes(name.as_bytes()).map_err(GcsDeliveryError::HeaderName)?,
                header_value,
            );
        }
        Ok(headers)
    }

    fn upload_url(&self, object_name: &str) -> String {
        format!(
            "{MEDIA_UPLOAD_ROOT}/{bucket}/o?uploadType=media&name={encoded_name}",
            bucket = self.bucket_name,
            encoded_name = encode_object_name(object_name, Encoding::Gcs),
        )
    }
}

impl<Auth, Http> FocusDestination for FocusGcsDestination<Auth, Http>
where
    Auth: VertexAuth + Clone + Debug + 'static,
    Http: GcsHttpClient + Debug + 'static,
{
    type Error = GcsDeliveryError<Auth, Http>;

    async fn deliver<'delivery>(
        &'delivery self,
        content: &'delivery [u8],
        time_window: &'delivery FocusTimeWindow,
        filename: &'delivery str,
    ) -> Result<(), Self::Error> {
        let object_name = object_key::build(&self.prefix, time_window, filename);
        let headers = self.request_headers().await?;
        let url = self.upload_url(&object_name);
        let response = self
            .base
            .client
            .send(GcsRequest::Post(GcsWriteRequest {
                url,
                data: None,
                json: None,
                params: BTreeMap::new(),
                headers,
                timeout: None,
                stream: false,
                content: Some(content.to_vec()),
            }))
            .await
            .map_err(GcsDeliveryError::Transport)?;
        if response.status != 200 {
            return Err(GcsDeliveryError::Status {
                status: response.status,
                body: from_utf8(&response.body).unwrap_or_default().to_owned(),
            });
        }
        tracing::debug!(
            "Focus GCS: uploaded {} bytes to gs://{}/{}",
            content.len(),
            self.bucket_name,
            object_name
        );
        Ok(())
    }
}
