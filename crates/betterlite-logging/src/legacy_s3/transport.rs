use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_http::proxy::ProxiedClient;

use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::signing::SystemClock;
use crate::s3_transport::transport::{HttpS3Client, S3Transport};
use crate::s3_transport::url_construction::{EndpointInputs, resolve_endpoint};

use super::credentials::ConfiguredCredentials;
use super::settings::Settings;

mod tests;

pub fn build(
    settings: &Settings,
    environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    client: ProxiedClient,
) -> Result<Option<S3Transport>, S3TransportError> {
    let bucket = settings
        .bucket_name
        .as_deref()
        .filter(|name| !name.is_empty());
    if bucket.is_none() {
        return Ok(None);
    }
    if let Some(endpoint) = settings.endpoint_url.as_ref() {
        url::Url::parse(endpoint).map_err(|source| S3TransportError::Url {
            url: endpoint.clone(),
            source,
        })?;
    }
    // the legacy logger has no s3_use_virtual_hosted_style, so a custom endpoint stays path style
    let endpoint = resolve_endpoint(
        EndpointInputs {
            bucket,
            region: settings.region_name.as_deref(),
            endpoint_url: settings.endpoint_url.as_deref(),
            virtual_hosted_style: false,
        },
        environment.as_ref(),
    )?;
    Ok(Some(S3Transport {
        client: Arc::new(HttpS3Client { client }),
        endpoint,
        credentials: Arc::new(ConfiguredCredentials {
            environment,
            access_key_id: settings.access_key_id.clone(),
            secret_access_key: settings.secret_access_key.clone(),
            session_token: settings.session_token.clone(),
        }),
        encryption: settings.encryption.clone(),
        // legacy S3 sizing is boto3's own, so a terminal rejection is final as in the v2 uploader
        drop_on_terminal_error: true,
        throttle: None,
        clock: Arc::new(SystemClock),
    }))
}
