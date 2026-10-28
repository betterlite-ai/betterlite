use std::sync::Arc;

use azure_core::Error as AzureError;
use azure_core::credentials::TokenCredential;
use azure_core::error::ErrorKind;
use azure_core::http::{ClientOptions, Transport, Url};
use azure_storage_blob::{BlobContainerClient, BlobServiceClient, BlobServiceClientOptions};
use betterlite_config::sections::azure_blob_cache::AzureBlobSettings;

use super::CacheError;

pub(super) fn build(
    settings: &AzureBlobSettings,
    credential: Arc<dyn TokenCredential>,
    transport: Transport,
) -> Result<BlobContainerClient, CacheError> {
    let url = Url::parse(&settings.account_url).map_err(|source| CacheError::InvalidUrl {
        source: AzureError::new(ErrorKind::DataConversion, source),
    })?;
    let options = BlobServiceClientOptions {
        client_options: ClientOptions {
            transport: Some(transport),
            ..Default::default()
        },
        ..Default::default()
    };
    let service_client = BlobServiceClient::new(url, Some(credential), Some(options))
        .map_err(|source| CacheError::Client { source })?;
    Ok(service_client.blob_container_client(&settings.container))
}
