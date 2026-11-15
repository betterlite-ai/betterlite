use std::error::Error;
use std::future::Future;

use secrecy::{ExposeSecret as _, SecretString};

use crate::focus_records::destinations::{FocusDestination, FocusTimeWindow};

use super::object_key;

pub const CONTENT_TYPE: &str = "application/octet-stream";

#[derive(Clone, Debug, Default)]
pub struct S3Settings {
    pub bucket_name: Option<String>,
    pub region_name: Option<String>,
    pub endpoint_url: Option<String>,
    pub aws_access_key_id: Option<SecretString>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
}

#[derive(Clone, Debug, Default)]
pub struct S3ClientOptions {
    pub region_name: Option<String>,
    pub endpoint_url: Option<String>,
    pub aws_access_key_id: Option<SecretString>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
}

#[derive(Debug)]
pub struct S3PutObject {
    pub bucket_name: String,
    pub object_key: String,
    pub body: Vec<u8>,
    pub content_type: &'static str,
}

pub trait S3Client: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    // The boto3 client is built per upload from the options, so each delivery passes them along
    fn put_object(
        &self,
        client_options: &S3ClientOptions,
        object: S3PutObject,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum S3SettingsError {
    #[error("bucket_name must be provided for the S3 FOCUS destination")]
    MissingBucketName,
}

#[derive(Debug, thiserror::Error)]
pub enum S3DeliveryError<UploadError>
where
    UploadError: Error + 'static,
{
    #[error("the S3 object could not be uploaded")]
    Upload {
        #[source]
        source: UploadError,
    },
}

pub struct FocusS3Destination<Client: S3Client> {
    pub(super) client: Client,
    pub(super) bucket_name: String,
    pub(super) prefix: String,
    pub(super) client_options: S3ClientOptions,
}

impl<Client: S3Client> FocusS3Destination<Client> {
    pub fn new(
        client: Client,
        prefix: &str,
        settings: S3Settings,
    ) -> Result<Self, S3SettingsError> {
        let Some(bucket_name) = settings.bucket_name.filter(|name| !name.is_empty()) else {
            return Err(S3SettingsError::MissingBucketName);
        };
        let client_options = S3ClientOptions {
            region_name: settings.region_name.filter(|value| !value.is_empty()),
            endpoint_url: settings.endpoint_url.filter(|value| !value.is_empty()),
            aws_access_key_id: settings
                .aws_access_key_id
                .filter(|value| !value.expose_secret().is_empty()),
            aws_secret_access_key: settings
                .aws_secret_access_key
                .filter(|value| !value.expose_secret().is_empty()),
            aws_session_token: settings
                .aws_session_token
                .filter(|value| !value.expose_secret().is_empty()),
        };
        Ok(Self {
            client,
            bucket_name,
            prefix: prefix.trim_end_matches('/').to_owned(),
            client_options,
        })
    }
}

impl<Client: S3Client> FocusDestination for FocusS3Destination<Client> {
    type Error = S3DeliveryError<Client::Error>;

    async fn deliver<'delivery>(
        &'delivery self,
        content: &'delivery [u8],
        time_window: &'delivery FocusTimeWindow,
        filename: &'delivery str,
    ) -> Result<(), Self::Error> {
        let object = S3PutObject {
            bucket_name: self.bucket_name.clone(),
            object_key: object_key::build(&self.prefix, time_window, filename),
            body: content.to_vec(),
            content_type: CONTENT_TYPE,
        };
        self.client
            .put_object(&self.client_options, object)
            .await
            .map_err(|source| S3DeliveryError::Upload { source })
    }
}
