use std::sync::PoisonError;
use std::sync::atomic::Ordering;
use std::time::Duration;

use http::Method;

use super::{
    AzureBlobStorageLogger, AzureStorageError, CachedClient, FIRST_ERROR_STATUS, SERVICE_CLIENT_TTL,
};
use crate::azure_storage_logging::http_ops::{StorageAuthorization, storage_request};

const NOT_FOUND: u16 = 404;
const SDK_FLUSH_QUERY: &str = "&retainUncommittedData=false&close=false";

// the SDK's directory exists() is a HEAD on the blob endpoint, and a 404 there means create it
pub(super) async fn ensure_directory(
    logger: &AzureBlobStorageLogger,
    exists_url: &str,
    directory_url: &str,
    auth: &StorageAuthorization<'_>,
) -> Result<(), AzureStorageError> {
    let now = logger.deps.clock.now_unix();
    let exists = logger
        .deps
        .transport
        .send(storage_request(
            Method::HEAD,
            exists_url.to_owned(),
            auth,
            now,
            &[],
        )?)
        .await?;
    if exists.status == NOT_FOUND {
        let create = storage_request(
            Method::PUT,
            format!("{directory_url}?resource=directory"),
            auth,
            now,
            &[],
        )?;
        let _made = logger.execute(create, "create_directory").await?;
        return Ok(());
    }
    if exists.status >= FIRST_ERROR_STATUS {
        return Err(AzureStorageError::Status {
            operation: "exists",
            status: exists.status,
        });
    }
    Ok(())
}

// create, append, flush; the SDK's flush adds retainUncommittedData and close to LiteLLM's REST flush
pub(super) async fn write_file(
    logger: &AzureBlobStorageLogger,
    base_url: &str,
    auth: &StorageAuthorization<'_>,
    content: &[u8],
) -> Result<(), AzureStorageError> {
    let now = logger.deps.clock.now_unix();
    let flush_query = match auth {
        StorageAuthorization::Bearer(_) => "",
        StorageAuthorization::SharedKey { .. } => SDK_FLUSH_QUERY,
    };
    let create = storage_request(
        Method::PUT,
        format!("{base_url}?resource=file"),
        auth,
        now,
        &[],
    )?;
    let _created = logger.execute(create, "create").await?;
    let append = storage_request(
        Method::PATCH,
        format!("{base_url}?action=append&position=0"),
        auth,
        now,
        content,
    )?;
    let _appended = logger.execute(append, "append").await?;
    let position = content.len();
    let flush = storage_request(
        Method::PATCH,
        format!("{base_url}?action=flush&position={position}{flush_query}"),
        auth,
        now,
        &[],
    )?;
    let _flushed = logger.execute(flush, "flush").await?;
    Ok(())
}

// the SDK client is rebuilt once its TTL passes, which is LiteLLM's connection-pool reset
pub(super) fn service_client(
    logger: &AzureBlobStorageLogger,
    now: Duration,
) -> Result<CachedClient, AzureStorageError> {
    let cached = logger
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .client
        .clone();
    if let Some(client) = cached {
        if client.expires_at > now {
            return Ok(client);
        }
        let _closed = logger.closed_clients.fetch_add(1, Ordering::SeqCst);
    }
    let key = logger
        .settings
        .account_key
        .clone()
        .ok_or(AzureStorageError::MissingAccountKey)?;
    let created = CachedClient {
        account: logger.settings.account_name.clone(),
        key,
        expires_at: now.saturating_add(SERVICE_CLIENT_TTL),
    };
    let _created = logger.created_clients.fetch_add(1, Ordering::SeqCst);
    logger
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .client = Some(created.clone());
    Ok(created)
}
