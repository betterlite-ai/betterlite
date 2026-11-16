use betterlite_logging::azure_storage_logging::http_ops::{StorageAuthorization, storage_request};
use http::Method;
use percent_encoding::utf8_percent_encode;
use secrecy::SecretString;
use url::Url;

use super::AzureBlobStorageBackend;
use super::error::AzureFileStorageError;
use super::naming::PATH_SAFE;
use super::request::{resource_exists, send};

pub(super) async fn download_file(
    backend: &AzureBlobStorageBackend,
    storage_url: &str,
) -> Result<Vec<u8>, AzureFileStorageError> {
    let file_path = blob_file_path(storage_url)?;
    if let Some(key) = backend.settings.account_key.as_ref() {
        return download_with_account_key(backend, storage_url, file_path, key).await;
    }
    download_with_token(backend, file_path).await
}

// urllib.parse.urlparse's path: the caller's bytes before the query or fragment, less the last
// segment's ;params, which urlparse splits off for https
fn raw_path(storage_url: &str) -> Option<&str> {
    let (_scheme, after_scheme) = storage_url.split_once("://")?;
    let path_start = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let path_and_rest = &after_scheme[path_start..];
    let path_end = path_and_rest
        .find(['?', '#'])
        .unwrap_or(path_and_rest.len());
    let path = &path_and_rest[..path_end];
    let last_segment_start = path.rfind('/').map_or(0, |slash| slash + 1);
    let last_segment = path.get(last_segment_start..).unwrap_or_default();
    match last_segment.find(';') {
        Some(params) => path.get(..last_segment_start.saturating_add(params)),
        None => Some(path),
    }
}

// hostname must carry .blob., and the path's first segment is the container, which is discarded
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
    let Some(raw) = raw_path(storage_url) else {
        return Err(AzureFileStorageError::InvalidUrl {
            url: storage_url.to_owned(),
        });
    };
    let path = raw.trim_start_matches('/');
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
    storage_url: &str,
    file_path: &str,
    key: &SecretString,
) -> Result<Vec<u8>, AzureFileStorageError> {
    let settings = &backend.settings;
    let now = backend.logger.deps.clock.now_unix();
    let file_system = utf8_percent_encode(&settings.file_system, PATH_SAFE).to_string();
    let auth = StorageAuthorization::SharedKey {
        account: settings.account_name.as_str(),
        key,
    };
    // FileSystemClient.exists and the file download both run through the SDK's blob client
    let exists_url = format!("{}/{file_system}?restype=container", settings.blob_endpoint);
    let file_system_exists = resource_exists(backend, Method::GET, exists_url, &auth, now).await?;
    if !file_system_exists {
        return Err(AzureFileStorageError::MissingFileSystem {
            file_system: settings.file_system.clone(),
        });
    }
    // the SDK's get_file_client strips the path's leading and trailing /, except a lone /
    let path = if file_path == "/" {
        file_path
    } else {
        file_path.trim_matches('/')
    };
    if path.is_empty() {
        return Err(AzureFileStorageError::InvalidUrlFormat {
            url: storage_url.to_owned(),
        });
    }
    let encoded_path = utf8_percent_encode(path, PATH_SAFE).to_string();
    let url = format!("{}/{file_system}/{encoded_path}", settings.blob_endpoint);
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
