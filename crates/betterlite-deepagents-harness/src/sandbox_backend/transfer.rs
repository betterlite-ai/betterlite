use betterlite_harness::sandbox::Sandbox;

use crate::sandbox_backend::SandboxBackend;
use crate::sandbox_backend::errors::BackendError;
use crate::sandbox_backend::protocol::{FileDownloadResponse, FileUploadResponse, TransferError};

pub(crate) async fn upload<S>(
    backend: &SandboxBackend<S>,
    files: &[(String, Vec<u8>)],
) -> Vec<FileUploadResponse>
where
    S: Sandbox,
{
    let mut responses = Vec::with_capacity(files.len());
    for (path, data) in files {
        let outcome = if backend.permissions.writable {
            match backend.write_bytes(path, data).await {
                Ok(()) => Ok(()),
                Err(BackendError::Sandbox(error)) => Err(TransferError::Sandbox(error.to_string())),
                Err(_) => Err(TransferError::InvalidPath),
            }
        } else {
            Err(TransferError::PermissionDenied)
        };
        responses.push(FileUploadResponse {
            path: path.clone(),
            outcome,
        });
    }
    responses
}

pub(crate) async fn download<S>(
    backend: &SandboxBackend<S>,
    paths: &[String],
) -> Vec<FileDownloadResponse>
where
    S: Sandbox,
{
    let mut responses = Vec::with_capacity(paths.len());
    for path in paths {
        let content = match backend.read_bytes(path).await {
            Ok(data) => Ok(data),
            Err(BackendError::Sandbox(_)) => Err(TransferError::FileNotFound),
            Err(_) => Err(TransferError::InvalidPath),
        };
        responses.push(FileDownloadResponse {
            path: path.clone(),
            content,
        });
    }
    responses
}
