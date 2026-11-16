use std::time::Duration;

use betterlite_logging::azure_storage_logging::http_ops::{StorageAuthorization, storage_request};
use betterlite_logging::azure_storage_logging::logger::FIRST_ERROR_STATUS;
use betterlite_logging::azure_storage_logging::transport::{StorageRequest, StorageResponse};
use http::Method;

use super::AzureBlobStorageBackend;
use super::error::AzureFileStorageError;

pub(super) const NOT_FOUND: u16 = 404;
const OCTET_STREAM: &str = "application/octet-stream";

pub(super) async fn send(
    backend: &AzureBlobStorageBackend,
    request: StorageRequest,
    operation: &'static str,
) -> Result<StorageResponse, AzureFileStorageError> {
    let response = backend.logger.deps.transport.send(request).await?;
    if response.status >= FIRST_ERROR_STATUS {
        return Err(AzureFileStorageError::Status {
            operation,
            status: response.status,
        });
    }
    Ok(response)
}

pub(super) async fn send_step(
    backend: &AzureBlobStorageBackend,
    method: Method,
    url: String,
    auth: &StorageAuthorization<'_>,
    now: Duration,
    body: &[u8],
    operation: &'static str,
) -> Result<(), AzureFileStorageError> {
    let request = storage_request(method, url, auth, now, body)?;
    let _response = send(backend, request, operation).await?;
    Ok(())
}

// the SDK's exists() calls: a 404 means create, any other error status propagates
pub(super) async fn resource_exists(
    backend: &AzureBlobStorageBackend,
    method: Method,
    url: String,
    auth: &StorageAuthorization<'_>,
    now: Duration,
) -> Result<bool, AzureFileStorageError> {
    let request = storage_request(method, url, auth, now, &[])?;
    let response = backend.logger.deps.transport.send(request).await?;
    match response.status {
        NOT_FOUND => Ok(false),
        status if status >= FIRST_ERROR_STATUS => Err(AzureFileStorageError::Status {
            operation: "exists",
            status,
        }),
        _ => Ok(true),
    }
}

// LiteLLM's file upload appends bytes, so the REST helper's JSON content type is replaced
pub(super) fn append_request(
    url: String,
    auth: &StorageAuthorization<'_>,
    now: Duration,
    body: &[u8],
) -> Result<StorageRequest, AzureFileStorageError> {
    let mut request = storage_request(Method::PATCH, url, auth, now, body)?;
    if let Some((_name, value)) = request
        .headers
        .iter_mut()
        .find(|(name, _value)| name.eq_ignore_ascii_case("content-type"))
    {
        *value = OCTET_STREAM.to_owned();
        return Ok(request);
    }
    request
        .headers
        .push(("Content-Type".to_owned(), OCTET_STREAM.to_owned()));
    Ok(request)
}
