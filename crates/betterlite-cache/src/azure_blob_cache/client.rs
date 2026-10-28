use std::sync::Arc;

use azure_core::Error as AzureError;
use azure_core::credentials::TokenCredential;
use azure_core::error::ErrorKind;
use azure_core::http::{
    ClientOptions, InstrumentationOptions, LoggingOptions, RetryOptions, Transport, Url,
    UserAgentOptions,
};
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
    let client_options = ClientOptions {
        per_call_policies: Vec::new(),
        per_try_policies: Vec::new(),
        retry: RetryOptions::default(),
        transport: Some(transport),
        user_agent: UserAgentOptions::default(),
        instrumentation: InstrumentationOptions::default(),
        logging: LoggingOptions::default(),
        cloud: None,
    };
    let options = BlobServiceClientOptions {
        client_options,
        version: BlobServiceClientOptions::default().version,
    };
    let service_client = BlobServiceClient::new(url, Some(credential), Some(options))
        .map_err(|source| CacheError::Client { source })?;
    Ok(service_client.blob_container_client(&settings.container))
}
