use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::api::file_azure_storage::Settings;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::file_azure_storage::error::AzureFileStorageError;
use betterlite_core::api::file_azure_storage::naming::naming_strategy;
use betterlite_core::api::file_azure_storage::{AzureBlobStorageBackend, AzureFileStorageDeps};
use betterlite_core::api::file_storage_contracts::storage::{FileStorageBackend, UploadOptions};
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::azure_storage_logging::filename::RandomIds;
use betterlite_logging::azure_storage_logging::token::{
    ChainError, ChainProviderFactory, ChainTokenProvider,
};
use betterlite_logging::azure_storage_logging::transport::PooledTransport;
use betterlite_logging::datadog_logging::clock::SystemClock;
use secrecy::SecretString;

use super::error::SdkHttpError;
use super::result::{FileStorageFailure, FileStorageRun, SdkResult};
use super::{FileAzureStorageCall, FileDownloadCall, FileUploadCall};

struct FixedToken;

impl ChainTokenProvider for FixedToken {
    fn token(&self) -> Result<SecretString, ChainError> {
        Ok(SecretString::from("tok-1"))
    }
}

struct FixedChain;

impl ChainProviderFactory for FixedChain {
    fn provider(&self) -> Result<Arc<dyn ChainTokenProvider>, ChainError> {
        Ok(Arc::new(FixedToken))
    }
}

pub(crate) async fn send(
    call: &FileAzureStorageCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let backend = backend(api_base, env)?;
    match call {
        FileAzureStorageCall::Upload(upload) => Ok(upload_answer(&backend, upload, api_base).await),
        FileAzureStorageCall::Download(download) => Ok(download_answer(&backend, download).await),
    }
}

// the configured origins point at the case's mock, and every other setting comes from the case env
fn backend(
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<Arc<AzureBlobStorageBackend>, SdkHttpError> {
    let mut settings = Settings::from_environment(env)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    settings.dfs_endpoint = format!("{api_base}/dfs");
    settings.blob_endpoint = format!("{api_base}/blob");
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    let client =
        ProxiedClient::outbound(&policy).map_err(|error| SdkHttpError::Client(Box::new(error)))?;
    Ok(AzureBlobStorageBackend::new(
        settings,
        AzureFileStorageDeps {
            transport: Arc::new(PooledTransport { client }),
            clock: Arc::new(SystemClock),
            environment: Arc::new(env.clone()),
            ids: Arc::new(RandomIds),
            chain_factory: Arc::new(FixedChain),
        },
    ))
}

async fn upload_answer(
    backend: &AzureBlobStorageBackend,
    upload: &FileUploadCall,
    api_base: &str,
) -> SdkResult {
    let options = UploadOptions {
        path_prefix: upload.path_prefix.as_deref(),
        file_naming_strategy: naming_strategy(&upload.file_naming_strategy),
    };
    match backend
        .upload_file(
            &upload.file_content,
            &upload.filename,
            &upload.content_type,
            options,
        )
        .await
    {
        Ok(url) => SdkResult::FileAzureStorage {
            value: FileStorageRun {
                url: Some(reported(&url, api_base)),
                downloaded: None,
                error: None,
            },
        },
        Err(error) => SdkResult::FileAzureStorage {
            value: FileStorageRun {
                url: None,
                downloaded: None,
                error: Some(failure(&error)),
            },
        },
    }
}

async fn download_answer(
    backend: &AzureBlobStorageBackend,
    download: &FileDownloadCall,
) -> SdkResult {
    match backend.download_file(&download.storage_url).await {
        Ok(content) => SdkResult::FileAzureStorage {
            value: FileStorageRun {
                url: None,
                downloaded: Some(content),
                error: None,
            },
        },
        Err(error) => SdkResult::FileAzureStorage {
            value: FileStorageRun {
                url: None,
                downloaded: None,
                error: Some(failure(&error)),
            },
        },
    }
}

fn reported(url: &str, api_base: &str) -> String {
    match url.strip_prefix(api_base) {
        Some(rest) => rest.to_owned(),
        None => url.to_owned(),
    }
}

fn failure(error: &AzureFileStorageError) -> FileStorageFailure {
    let status = if let AzureFileStorageError::Status { status, .. } = error {
        Some(*status)
    } else {
        None
    };
    FileStorageFailure {
        message: error.to_string(),
        status,
    }
}
