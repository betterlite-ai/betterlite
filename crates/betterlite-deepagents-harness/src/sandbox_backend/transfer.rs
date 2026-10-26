use betterlite_harness::errors::HarnessError;
use betterlite_harness::sandbox::Sandbox;

use crate::sandbox_backend::SandboxBackend;
use crate::sandbox_backend::errors::BackendError;
use crate::sandbox_backend::protocol::{FileUploadResponse, TransferError};

pub(crate) async fn upload<S>(
    backend: &SandboxBackend<S>,
    (path, data): &(String, Vec<u8>),
) -> FileUploadResponse
where
    S: Sandbox,
{
    let outcome = if backend.permissions.writable {
        backend.write_bytes(path, data).await.map_err(|error| {
            transfer_error(error, |source| TransferError::Sandbox(source.to_string()))
        })
    } else {
        Err(TransferError::PermissionDenied)
    };
    FileUploadResponse {
        path: path.clone(),
        outcome,
    }
}

pub(crate) fn transfer_error(
    error: BackendError,
    sandbox: fn(HarnessError) -> TransferError,
) -> TransferError {
    match error {
        BackendError::Sandbox(source) => sandbox(source),
        BackendError::NotFound { .. }
        | BackendError::OutsideWorkspace { .. }
        | BackendError::Glob(_)
        | BackendError::GrepLine(_)
        | BackendError::EventLoopThread => TransferError::InvalidPath,
    }
}
