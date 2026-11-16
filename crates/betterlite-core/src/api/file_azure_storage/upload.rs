use std::time::Duration;

use betterlite_logging::azure_storage_logging::http_ops::{StorageAuthorization, storage_request};
use http::Method;
use percent_encoding::utf8_percent_encode;
use secrecy::SecretString;

use super::AzureBlobStorageBackend;
use super::error::AzureFileStorageError;
use super::naming::{PATH_SAFE, generated_file_name};
use super::request::{append_request, resource_exists, send};
use crate::api::file_storage_contracts::storage::UploadOptions;

pub(super) async fn upload_file(
    backend: &AzureBlobStorageBackend,
    content: &[u8],
    filename: &str,
    options: UploadOptions<'_>,
) -> Result<String, AzureFileStorageError> {
    let now = backend.logger.deps.clock.now_unix();
    let file_name = generated_file_name(
        filename,
        options.file_naming_strategy,
        now,
        backend.logger.deps.ids.as_ref(),
    );
    let full_path = options.path_prefix.map_or_else(
        || file_name.clone(),
        |prefix| format!("{}/{file_name}", prefix.trim_matches('/')),
    );
    if let Some(key) = backend.settings.account_key.as_ref() {
        return upload_with_account_key(backend, content, &full_path, key, now).await;
    }
    upload_with_token(backend, content, &full_path, now).await
}

async fn upload_with_account_key(
    backend: &AzureBlobStorageBackend,
    content: &[u8],
    full_path: &str,
    key: &SecretString,
    now: Duration,
) -> Result<String, AzureFileStorageError> {
    let settings = &backend.settings;
    let auth = StorageAuthorization::SharedKey {
        account: settings.account_name.as_str(),
        key,
    };
    ensure_file_system(backend, &auth, now).await?;
    let file_url = ensure_file_url(backend, full_path, &auth, now).await?;
    write_file(backend, &file_url, content, &auth, now).await?;
    Ok(format!(
        "{}/{}/{}",
        settings.blob_endpoint, settings.file_system, full_path
    ))
}

// the SDK's FileSystemClient.exists and create_file_system both run through its blob client
async fn ensure_file_system(
    backend: &AzureBlobStorageBackend,
    auth: &StorageAuthorization<'_>,
    now: Duration,
) -> Result<(), AzureFileStorageError> {
    let settings = &backend.settings;
    let file_system = utf8_percent_encode(&settings.file_system, PATH_SAFE).to_string();
    let exists_url = format!("{}/{file_system}?restype=container", settings.blob_endpoint);
    let file_system_exists =
        resource_exists(backend, Method::GET, exists_url.clone(), auth, now).await?;
    if !file_system_exists {
        let request = storage_request(Method::PUT, exists_url, auth, now, &[])?;
        send(backend, request, "create_file_system").await?;
    }
    Ok(())
}

// the SDK checks a directory through its blob client and creates it through its data lake client
async fn ensure_directory(
    backend: &AzureBlobStorageBackend,
    directory: &str,
    auth: &StorageAuthorization<'_>,
    now: Duration,
) -> Result<(), AzureFileStorageError> {
    let settings = &backend.settings;
    let file_system = utf8_percent_encode(&settings.file_system, PATH_SAFE).to_string();
    let encoded = utf8_percent_encode(directory, PATH_SAFE).to_string();
    let exists_url = format!("{}/{file_system}/{encoded}", settings.blob_endpoint);
    let directory_exists = resource_exists(backend, Method::HEAD, exists_url, auth, now).await?;
    if !directory_exists {
        let url = format!(
            "{}/{file_system}/{encoded}?resource=directory",
            settings.dfs_endpoint
        );
        let request = storage_request(Method::PUT, url, auth, now, &[])?;
        send(backend, request, "create_directory").await?;
    }
    Ok(())
}

// LiteLLM hands the SDK an empty directory name, which is never a valid data lake path
async fn ensure_file_url(
    backend: &AzureBlobStorageBackend,
    full_path: &str,
    auth: &StorageAuthorization<'_>,
    now: Duration,
) -> Result<String, AzureFileStorageError> {
    let settings = &backend.settings;
    let file_system = utf8_percent_encode(&settings.file_system, PATH_SAFE).to_string();
    let Some((directory, file_name)) = full_path.rsplit_once('/') else {
        return Ok(format!(
            "{}/{file_system}/{}",
            settings.dfs_endpoint,
            utf8_percent_encode(full_path, PATH_SAFE)
        ));
    };
    if directory.is_empty() {
        return Err(AzureFileStorageError::EmptyDirectory {
            path: full_path.to_owned(),
        });
    }
    ensure_directory(backend, directory, auth, now).await?;
    Ok(format!(
        "{}/{file_system}/{}/{}",
        settings.dfs_endpoint,
        utf8_percent_encode(directory, PATH_SAFE),
        utf8_percent_encode(file_name, PATH_SAFE)
    ))
}

// create_file, append and flush; the SDK's flush adds retainUncommittedData and close
async fn write_file(
    backend: &AzureBlobStorageBackend,
    base_url: &str,
    content: &[u8],
    auth: &StorageAuthorization<'_>,
    now: Duration,
) -> Result<(), AzureFileStorageError> {
    let create_url = format!("{base_url}?resource=file");
    let create_request = storage_request(Method::PUT, create_url, auth, now, &[])?;
    send(backend, create_request, "create_file").await?;
    let append_url = format!("{base_url}?action=append&position=0");
    match auth {
        StorageAuthorization::Bearer(_) => {
            let append = append_request(append_url, auth, now, content)?;
            send(backend, append, "append").await?;
        }
        StorageAuthorization::SharedKey { .. } => {
            let append = storage_request(Method::PATCH, append_url, auth, now, content)?;
            send(backend, append, "append").await?;
        }
    }
    let position = content.len();
    let flush_query = match auth {
        StorageAuthorization::Bearer(_) => "",
        StorageAuthorization::SharedKey { .. } => "&retainUncommittedData=false&close=false",
    };
    let flush_url = format!("{base_url}?action=flush&position={position}{flush_query}");
    let flush_request = storage_request(Method::PATCH, flush_url, auth, now, &[])?;
    send(backend, flush_request, "flush").await?;
    Ok(())
}

async fn upload_with_token(
    backend: &AzureBlobStorageBackend,
    content: &[u8],
    full_path: &str,
    now: Duration,
) -> Result<String, AzureFileStorageError> {
    let settings = &backend.settings;
    let token = backend.logger.set_valid_azure_ad_token().await?;
    let auth = StorageAuthorization::Bearer(&token);
    let base_url = format!(
        "{}/{}/{full_path}",
        settings.dfs_endpoint, settings.file_system
    );
    write_file(backend, &base_url, content, &auth, now).await?;
    Ok(format!(
        "{}/{}/{}",
        settings.blob_endpoint, settings.file_system, full_path
    ))
}
