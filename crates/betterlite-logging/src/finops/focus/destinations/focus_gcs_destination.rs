use std::error::Error;
use std::future::Future;

use betterlite_context::cloud_storage::{Encoding, encode_object_name};
use secrecy::SecretString;

use crate::focus_records::destinations::{FocusDestination, FocusTimeWindow};
use crate::gcs_client::gcs_storage_auth::{CONTENT_TYPE_HEADER, RequestHeaders};

use super::object_key;

pub const MEDIA_UPLOAD_ROOT: &str = "https://storage.googleapis.com/upload/storage/v1/b";
pub const MEDIA_CONTENT_TYPE: &str = "application/octet-stream";

#[derive(Debug, Default)]
pub struct GcsSettings {
    pub bucket_name: Option<String>,
    pub service_account_json: Option<SecretString>,
}

#[derive(Debug)]
pub struct GcsMediaRequest {
    pub url: String,
    pub headers: RequestHeaders,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GcsMediaResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait GcsBucketClient: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn construct_request_headers(
        &self,
        bucket_name: &str,
        service_account_json: Option<&SecretString>,
    ) -> impl Future<Output = Result<RequestHeaders, Self::Error>> + Send;

    fn post_media(
        &self,
        request: GcsMediaRequest,
    ) -> impl Future<Output = Result<GcsMediaResponse, Self::Error>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum GcsSettingsError {
    #[error("bucket_name must be provided for the GCS FOCUS destination")]
    MissingBucketName,
}

#[derive(Debug, thiserror::Error)]
pub enum GcsDeliveryError<ClientError>
where
    ClientError: Error + 'static,
{
    #[error("the GCS request headers could not be constructed")]
    Headers {
        #[source]
        source: ClientError,
    },
    #[error("the GCS media upload could not be sent")]
    Transport {
        #[source]
        source: ClientError,
    },
    #[error("the GCS media upload was refused with status {status}: {body}")]
    Status { status: u16, body: String },
}

pub struct FocusGcsDestination<Client: GcsBucketClient> {
    pub client: Client,
    pub bucket_name: String,
    pub prefix: String,
    pub service_account_json: Option<SecretString>,
}

impl<Client: GcsBucketClient> FocusGcsDestination<Client> {
    pub fn new(
        client: Client,
        prefix: &str,
        settings: GcsSettings,
    ) -> Result<Self, GcsSettingsError> {
        let Some(bucket_name) = settings.bucket_name.filter(|name| !name.is_empty()) else {
            return Err(GcsSettingsError::MissingBucketName);
        };
        Ok(Self {
            client,
            bucket_name,
            prefix: prefix.trim_end_matches('/').to_owned(),
            service_account_json: settings.service_account_json,
        })
    }
}

impl<Client: GcsBucketClient> FocusDestination for FocusGcsDestination<Client> {
    type Error = GcsDeliveryError<Client::Error>;

    async fn deliver<'delivery>(
        &'delivery self,
        content: &'delivery [u8],
        time_window: &'delivery FocusTimeWindow,
        filename: &'delivery str,
    ) -> Result<(), Self::Error> {
        let object_name = object_key::build(&self.prefix, time_window, filename);
        let mut headers = self
            .client
            .construct_request_headers(&self.bucket_name, self.service_account_json.as_ref())
            .await
            .map_err(|source| GcsDeliveryError::Headers { source })?;
        // LiteLLM replaces the application/json type the GCS base sets with the media type
        headers.insert(
            CONTENT_TYPE_HEADER.to_owned(),
            SecretString::from(MEDIA_CONTENT_TYPE),
        );
        let encoded_name = encode_object_name(&object_name, Encoding::Gcs);
        let url = format!(
            "{MEDIA_UPLOAD_ROOT}/{}/o?uploadType=media&name={encoded_name}",
            self.bucket_name
        );
        let response = self
            .client
            .post_media(GcsMediaRequest {
                url,
                headers,
                body: content.to_vec(),
            })
            .await
            .map_err(|source| GcsDeliveryError::Transport { source })?;
        if response.status != 200 {
            return Err(GcsDeliveryError::Status {
                status: response.status,
                body: str::from_utf8(&response.body)
                    .unwrap_or_default()
                    .to_owned(),
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
