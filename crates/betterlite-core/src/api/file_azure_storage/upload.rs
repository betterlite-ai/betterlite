use std::time::Duration;

use betterlite_logging::azure_storage_logging::http_ops::StorageAuthorization;
use http::Method;
use secrecy::SecretString;

use super::AzureBlobStorageBackend;
use super::error::AzureFileStorageError;
use super::naming::{generated_file_name, quoted_path};
use super::request::{append_request, resource_exists, send, send_step};
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
    let dfs = settings.dfs_endpoint.as_str();
    let file_system = quoted_path(&settings.file_system);
    let auth = StorageAuthorization::SharedKey {
        account: settings.account_name.as_str(),
        key,
    };
    let exists_url = format!("{dfs}/{file_system}?restype=container");
    let exists = resource_exists(backend, Method::GET, exists_url.clone(), &auth, now).await?;
    if !exists {
        send_step(
            backend,
            Method::PUT,
            exists_url,
            &auth,
            now,
            &[],
            "create_file_system",
        )
        .await?;
    }
    let file_url = match full_path.rsplit_once('/') {
        Some((directory, file_name)) => {
            if directory.is_empty() {
                return Err(AzureFileStorageError::EmptyDirectory {
                    path: full_path.to_owned(),
                });
            }
            let encoded_directory = quoted_path(directory);
            let directory_url = format!("{dfs}/{file_system}/{encoded_directory}");
            let exists =
                resource_exists(backend, Method::HEAD, directory_url.clone(), &auth, now).await?;
            if !exists {
                send_step(
                    backend,
                    Method::PUT,
                    format!("{directory_url}?resource=directory"),
                    &auth,
                    now,
                    &[],
                    "create_directory",
                )
                .await?;
            }
            format!("{directory_url}/{}", quoted_path(file_name))
        }
        None => format!("{dfs}/{file_system}/{}", quoted_path(full_path)),
    };
    let base_url = file_url.as_str();
    send_step(
        backend,
        Method::PUT,
        format!("{base_url}?resource=file"),
        &auth,
        now,
        &[],
        "create_file",
    )
    .await?;
    send_step(
        backend,
        Method::PATCH,
        format!("{base_url}?action=append&position=0"),
        &auth,
        now,
        content,
        "append",
    )
    .await?;
    let position = content.len();
    send_step(
        backend,
        Method::PATCH,
        format!(
            "{base_url}?action=flush&position={position}&retainUncommittedData=false&close=false"
        ),
        &auth,
        now,
        &[],
        "flush",
    )
    .await?;
    Ok(format!(
        "{}/{}/{}",
        settings.blob_endpoint, settings.file_system, full_path
    ))
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
    send_step(
        backend,
        Method::PUT,
        format!("{base_url}?resource=file"),
        &auth,
        now,
        &[],
        "create_file",
    )
    .await?;
    let append = append_request(
        format!("{base_url}?action=append&position=0"),
        &auth,
        now,
        content,
    )?;
    let _appended = send(backend, append, "append").await?;
    let position = content.len();
    send_step(
        backend,
        Method::PATCH,
        format!("{base_url}?action=flush&position={position}"),
        &auth,
        now,
        &[],
        "flush",
    )
    .await?;
    Ok(format!(
        "{}/{}/{}",
        settings.blob_endpoint, settings.file_system, full_path
    ))
}
