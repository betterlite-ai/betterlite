use std::collections::BTreeMap;

use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_logging::azure_storage_logging::filename::IdSource;
use betterlite_logging::datadog_logging::clock::Clock;
use betterlite_openai::files::records::FilesPurpose;

use crate::api::file_storage_contracts::storage::FileContentRepository;

use super::error::{StorageBackendError, StorageServiceError};
use super::factory::{StorageBackendDeps, get_storage_backend};
use super::projection::{FileMetadata, StoredFile, create_file_object_with_storage_metadata};

pub const DEFAULT_FILENAME: &str = "file";
pub const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";
// no managed-file hook reaches this build, so a target upload always stops with this message
const TARGET_MODELS_UNAVAILABLE: &str = concat!(
    "Uploading with target_model_names requires a database-connected proxy, ",
    "and this proxy has no database configured",
);

pub struct StorageUpload<'data> {
    pub content: &'data [u8],
    pub filename: Option<&'data str>,
    pub content_type: Option<&'data str>,
    pub purpose: FilesPurpose,
    pub target_storage: &'data str,
    pub target_model_names: &'data [String],
}

impl<'data> StorageUpload<'data> {
    #[must_use]
    pub const fn filename(&self) -> &'data str {
        match self.filename {
            Some(filename) => filename,
            None => DEFAULT_FILENAME,
        }
    }

    #[must_use]
    pub const fn content_type(&self) -> &'data str {
        match self.content_type {
            Some(content_type) => content_type,
            None => DEFAULT_CONTENT_TYPE,
        }
    }
}

pub struct StorageUploadDeps<'deps, R> {
    pub backend: StorageBackendDeps<R>,
    pub clock: &'deps dyn Clock,
    pub ids: &'deps dyn IdSource,
}

pub async fn upload_file_to_storage_backend<R>(
    upload: StorageUpload<'_>,
    deps: StorageUploadDeps<'_, R>,
) -> Result<StoredFile, StorageServiceError>
where
    R: FileContentRepository,
{
    let backend =
        get_storage_backend(upload.target_storage, deps.backend).map_err(factory_failure)?;
    if !upload.target_model_names.is_empty() {
        return Err(StorageServiceError::Proxy(
            Box::new(target_models_failure()),
        ));
    }
    let storage_url = backend
        .upload(upload.content, upload.filename(), upload.content_type())
        .await?;
    let target_storage = upload.target_storage;
    tracing::debug!("Storage backend upload complete: backend={target_storage}, url={storage_url}");
    Ok(create_file_object_with_storage_metadata(
        FileMetadata {
            content: upload.content,
            filename: upload.filename(),
            purpose: upload.purpose,
            target_storage,
            storage_url: &storage_url,
        },
        deps.clock,
        deps.ids,
    ))
}

fn factory_failure(error: StorageBackendError) -> StorageServiceError {
    let message = error.to_string();
    StorageServiceError::Proxy(Box::new(invalid_request(message, "target_storage")))
}

fn target_models_failure() -> ProxyException {
    invalid_request(TARGET_MODELS_UNAVAILABLE.to_owned(), "target_model_names")
}

fn invalid_request(message: String, param: &str) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message,
        error_type: "invalid_request_error".to_owned(),
        param: Some(param.to_owned()),
        code: ExceptionCode::Status(400),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}
