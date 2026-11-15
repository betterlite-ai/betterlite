use std::str::from_utf8;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use http::Method;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

use crate::focus_records::destinations::{FocusDestination, FocusTimeWindow};
use crate::legacy_s3::credentials::ConfiguredCredentials;
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::signing::{Clock, CredentialSource as _, RequestToSign, sign_request};
use crate::s3_transport::transport::{S3HttpClient, S3Request};
use crate::s3_transport::url_construction::{EndpointInputs, resolve_endpoint};

use super::object_key;

pub const CONTENT_TYPE: &str = "application/octet-stream";
// boto3 reads the default region from this variable before any config file
pub const DEFAULT_REGION_VARIABLE: &str = "AWS_DEFAULT_REGION";

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

pub struct S3Connection {
    pub http: Arc<dyn S3HttpClient>,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub clock: Arc<dyn Clock>,
}

#[derive(Debug, thiserror::Error)]
pub enum S3SettingsError {
    #[error("bucket_name must be provided for the S3 FOCUS destination")]
    MissingBucketName,
}

#[derive(Debug, thiserror::Error)]
pub enum S3DeliveryError {
    #[error("the S3 object could not be uploaded")]
    Transport(#[from] S3TransportError),
    #[error("the S3 upload was refused with status {status}: {body}")]
    Status { status: u16, body: String },
}

pub struct FocusS3Destination {
    pub(super) connection: S3Connection,
    pub(super) bucket_name: String,
    pub(super) prefix: String,
    pub(super) client_options: S3ClientOptions,
}

impl FocusS3Destination {
    pub fn new(
        connection: S3Connection,
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
            connection,
            bucket_name,
            prefix: prefix.trim_end_matches('/').to_owned(),
            client_options,
        })
    }
}

impl FocusDestination for FocusS3Destination {
    type Error = S3DeliveryError;

    async fn deliver<'delivery>(
        &'delivery self,
        content: &'delivery [u8],
        time_window: &'delivery FocusTimeWindow,
        filename: &'delivery str,
    ) -> Result<(), Self::Error> {
        let object_key = object_key::build(&self.prefix, time_window, filename);
        let environment = &self.connection.environment;
        let options = &self.client_options;
        // boto3 builds its client for each upload, so the options are resolved here too
        let region = options.region_name.clone().or_else(|| {
            environment
                .lookup(DEFAULT_REGION_VARIABLE)
                .filter(|value| !value.is_empty())
        });
        let endpoint = resolve_endpoint(
            EndpointInputs {
                bucket: Some(&self.bucket_name),
                region: region.as_deref(),
                endpoint_url: options
                    .endpoint_url
                    .as_deref()
                    .map(|url| url.trim_end_matches('/')),
                virtual_hosted_style: false,
            },
            environment.as_ref(),
        )?;
        let credentials = ConfiguredCredentials {
            environment: Arc::clone(environment),
            access_key_id: options.aws_access_key_id.clone(),
            secret_access_key: options.aws_secret_access_key.clone(),
            session_token: options.aws_session_token.clone(),
        }
        .resolve()
        .await
        .map_err(S3TransportError::from)?;
        let url = endpoint.object_url(&object_key);
        let content_sha256 = hex::encode(Sha256::digest(content));
        let mut headers = vec![
            ("Content-Type".to_owned(), CONTENT_TYPE.to_owned()),
            ("x-amz-content-sha256".to_owned(), content_sha256.clone()),
        ];
        let signed = sign_request(
            &RequestToSign {
                method: "PUT",
                url: &url,
                headers: &headers,
                content_sha256: &content_sha256,
                region: &endpoint.region,
                now: self.connection.clock.now(),
            },
            &credentials,
        )?;
        headers.extend(signed);
        let response = self
            .connection
            .http
            .send(S3Request {
                method: Method::PUT,
                url,
                headers,
                body: content.to_vec(),
            })
            .await?;
        if !(200..300).contains(&response.status) {
            return Err(S3DeliveryError::Status {
                status: response.status,
                body: from_utf8(&response.body).unwrap_or_default().to_owned(),
            });
        }
        Ok(())
    }
}
