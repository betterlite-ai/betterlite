use std::time::Duration;

use betterlite_logging::azure_storage_logging::http_ops::{StorageAuthorization, storage_request};
use http::Method;
use percent_encoding::utf8_percent_encode;

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
    let object_name = generated_file_name(
        filename,
        options.file_naming_strategy,
        now,
        backend.logger.deps.ids.as_ref(),
    );
    let full_path = options.path_prefix.map_or_else(
        || object_name.clone(),
        |prefix| format!("{}/{object_name}", prefix.trim_matches('/')),
    );
    if let Some(key) = backend.settings.account_key.as_ref() {
        // the SDK checks the filesystem and the directory before it writes through them
        let auth = StorageAuthorization::SharedKey {
            account: backend.settings.account_name.as_str(),
            key,
        };
        ensure_target(backend, StorageTarget::FileSystem, &auth, now).await?;
        let file_url = ensure_file_url(backend, &full_path, &auth, now).await?;
        write_file(backend, &file_url, content, &auth, now).await?;
    } else {
        let token = backend.logger.set_valid_azure_ad_token().await?;
        let auth = StorageAuthorization::Bearer(&token);
        let target = format!(
            "{}/{}/{full_path}",
            backend.settings.dfs_endpoint, backend.settings.file_system
        );
        write_file(backend, &target, content, &auth, now).await?;
    }
    Ok(format!(
        "{}/{}/{}",
        backend.settings.blob_endpoint, backend.settings.file_system, full_path
    ))
}

#[derive(Clone, Copy)]
enum StorageTarget<'path> {
    FileSystem,
    Directory(&'path str),
}

// the SDK checks both targets through its blob client, creating a directory through its data lake client
async fn ensure_target(
    backend: &AzureBlobStorageBackend,
    target: StorageTarget<'_>,
    auth: &StorageAuthorization<'_>,
    now: Duration,
) -> Result<(), AzureFileStorageError> {
    let settings = &backend.settings;
    let file_system = utf8_percent_encode(&settings.file_system, PATH_SAFE).to_string();
    let (method, exists_url, create_url, operation) = match target {
        StorageTarget::FileSystem => {
            let container = format!("{}/{file_system}?restype=container", settings.blob_endpoint);
            (
                Method::GET,
                container.clone(),
                container,
                "create_file_system",
            )
        }
        StorageTarget::Directory(directory) => (
            Method::HEAD,
            format!(
                "{}/{file_system}/{}",
                settings.blob_endpoint,
                utf8_percent_encode(directory, PATH_SAFE)
            ),
            format!(
                "{}/{file_system}/{}?resource=directory",
                settings.dfs_endpoint,
                utf8_percent_encode(directory, PATH_SAFE)
            ),
            "create_directory",
        ),
    };
    let exists = resource_exists(backend, method, exists_url, auth, now).await?;
    if !exists {
        let request = storage_request(Method::PUT, create_url, auth, now, &[])?;
        send(backend, request, operation).await?;
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
    ensure_target(backend, StorageTarget::Directory(directory), auth, now).await?;
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
