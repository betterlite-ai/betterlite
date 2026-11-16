use betterlite_logging::azure_storage_logging::http_ops::{StorageAuthorization, storage_request};
use http::Method;
use secrecy::SecretString;
use url::Url;

use super::AzureBlobStorageBackend;
use super::error::AzureFileStorageError;
use super::naming::quoted_path;
use super::request::{resource_exists, send};

pub(super) async fn download_file(
    backend: &AzureBlobStorageBackend,
    storage_url: &str,
) -> Result<Vec<u8>, AzureFileStorageError> {
    let file_path = blob_file_path(storage_url)?;
    if let Some(key) = backend.settings.account_key.as_ref() {
        return download_with_account_key(backend, file_path, key).await;
    }
    download_with_token(backend, file_path).await
}

// urlparse(url).hostname must carry .blob., and the path's first segment is the container, which is discarded
fn blob_file_path(storage_url: &str) -> Result<&str, AzureFileStorageError> {
    let Ok(parsed) = Url::parse(storage_url) else {
        return Err(AzureFileStorageError::InvalidUrl {
            url: storage_url.to_owned(),
        });
    };
    if !parsed.host_str().unwrap_or_default().contains(".blob.") {
        return Err(AzureFileStorageError::InvalidUrl {
            url: storage_url.to_owned(),
        });
    }
    let path = parsed.path().trim_start_matches('/');
    let (_container, rest) = path.split_once('/').unwrap_or((path, ""));
    if rest.is_empty() {
        return Err(AzureFileStorageError::InvalidUrlFormat {
            url: storage_url.to_owned(),
        });
    }
    Ok(rest)
}

async fn download_with_account_key(
    backend: &AzureBlobStorageBackend,
    file_path: &str,
    key: &SecretString,
) -> Result<Vec<u8>, AzureFileStorageError> {
    let settings = &backend.settings;
    let now = backend.logger.deps.clock.now_unix();
    let file_system = quoted_path(&settings.file_system);
    let auth = StorageAuthorization::SharedKey {
        account: settings.account_name.as_str(),
        key,
    };
    let exists_url = format!("{}/{file_system}?restype=container", settings.dfs_endpoint);
    let exists = resource_exists(backend, Method::GET, exists_url, &auth, now).await?;
    if !exists {
        return Err(AzureFileStorageError::MissingFileSystem {
            file_system: settings.file_system.clone(),
        });
    }
    let url = format!(
        "{}/{file_system}/{}",
        settings.dfs_endpoint,
        quoted_path(file_path)
    );
    let request = storage_request(Method::GET, url, &auth, now, &[])?;
    Ok(send(backend, request, "download").await?.body)
}

async fn download_with_token(
    backend: &AzureBlobStorageBackend,
    file_path: &str,
) -> Result<Vec<u8>, AzureFileStorageError> {
    let settings = &backend.settings;
    let token = backend.logger.set_valid_azure_ad_token().await?;
    let auth = StorageAuthorization::Bearer(&token);
    let url = format!(
        "{}/{}/{file_path}",
        settings.blob_endpoint, settings.file_system
    );
    let now = backend.logger.deps.clock.now_unix();
    let request = storage_request(Method::GET, url, &auth, now, &[])?;
    Ok(send(backend, request, "download").await?.body)
}
