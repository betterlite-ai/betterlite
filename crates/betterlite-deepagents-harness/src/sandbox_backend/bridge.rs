use std::future::Future;

use betterlite_harness::sandbox::Sandbox;
use futures_util::{StreamExt as _, stream};
use tokio::runtime::Handle;

use crate::sandbox_backend::errors::BackendError;
use crate::sandbox_backend::protocol::{
    DeleteResult, EditResult, ExecuteResponse, FileDownloadResponse, FileUploadResponse,
    GlobResult, GrepQuery, GrepResult, LsResult, ReadResult, Replacement, Reply, WriteResult,
};
use crate::sandbox_backend::{SandboxBackend, execute, files, search, transfer};

pub trait SandboxBackendProtocol {
    fn id(&self) -> &str;
    fn block_on<F>(&self, operation: F) -> Result<F::Output, BackendError>
    where
        F: Future;
    fn ls(&self, path: &str) -> impl Future<Output = LsResult> + Send;
    fn read(
        &self,
        path: &str,
        offset: usize,
        limit: usize,
    ) -> impl Future<Output = ReadResult> + Send;
    fn write(&self, path: &str, content: &str) -> impl Future<Output = WriteResult> + Send;
    fn edit(&self, path: &str, change: Replacement<'_>) -> impl Future<Output = EditResult> + Send;
    fn delete(&self, path: &str) -> impl Future<Output = DeleteResult> + Send;
    fn glob(&self, pattern: &str, path: &str) -> impl Future<Output = GlobResult> + Send;
    fn grep(&self, query: GrepQuery<'_>) -> impl Future<Output = GrepResult> + Send;
    fn upload_files(
        &self,
        files: &[(String, Vec<u8>)],
    ) -> impl Future<Output = Vec<FileUploadResponse>> + Send;
    fn download_files(
        &self,
        paths: &[String],
    ) -> impl Future<Output = Vec<FileDownloadResponse>> + Send;
    fn execute(
        &self,
        command: &str,
        timeout_seconds: Option<u64>,
    ) -> impl Future<Output = ExecuteResponse> + Send;

    fn blocking_ls(&self, path: &str) -> Result<LsResult, BackendError> {
        self.block_on(self.ls(path))
    }

    fn blocking_read(
        &self,
        path: &str,
        offset: usize,
        limit: usize,
    ) -> Result<ReadResult, BackendError> {
        self.block_on(self.read(path, offset, limit))
    }

    fn blocking_write(&self, path: &str, content: &str) -> Result<WriteResult, BackendError> {
        self.block_on(self.write(path, content))
    }

    fn blocking_edit(
        &self,
        path: &str,
        change: Replacement<'_>,
    ) -> Result<EditResult, BackendError> {
        self.block_on(self.edit(path, change))
    }

    fn blocking_delete(&self, path: &str) -> Result<DeleteResult, BackendError> {
        self.block_on(self.delete(path))
    }

    fn blocking_glob(&self, pattern: &str, path: &str) -> Result<GlobResult, BackendError> {
        self.block_on(self.glob(pattern, path))
    }

    fn blocking_grep(&self, query: GrepQuery<'_>) -> Result<GrepResult, BackendError> {
        self.block_on(self.grep(query))
    }

    fn blocking_upload_files(
        &self,
        files: &[(String, Vec<u8>)],
    ) -> Result<Vec<FileUploadResponse>, BackendError> {
        self.block_on(self.upload_files(files))
    }

    fn blocking_download_files(
        &self,
        paths: &[String],
    ) -> Result<Vec<FileDownloadResponse>, BackendError> {
        self.block_on(self.download_files(paths))
    }

    fn blocking_execute(
        &self,
        command: &str,
        timeout_seconds: Option<u64>,
    ) -> Result<ExecuteResponse, BackendError> {
        self.block_on(self.execute(command, timeout_seconds))
    }
}

impl<S> SandboxBackendProtocol for SandboxBackend<S>
where
    S: Sandbox,
{
    fn id(&self) -> &str {
        &self.id
    }

    // Handle::block_on panics inside any runtime, so every runtime thread is refused, not only this one
    fn block_on<F>(&self, operation: F) -> Result<F::Output, BackendError>
    where
        F: Future,
    {
        if Handle::try_current().is_ok() {
            return Err(BackendError::EventLoopThread);
        }
        Ok(self.runtime.block_on(operation))
    }

    fn ls(&self, path: &str) -> impl Future<Output = LsResult> + Send {
        files::list(self, path)
    }

    fn read(
        &self,
        path: &str,
        offset: usize,
        limit: usize,
    ) -> impl Future<Output = ReadResult> + Send {
        files::read(self, path, offset, limit)
    }

    fn write(&self, path: &str, content: &str) -> impl Future<Output = WriteResult> + Send {
        files::when_writable(self, async move {
            match self.write_bytes(path, content.as_bytes()).await {
                Ok(()) => Reply::Done(path.to_owned()),
                Err(error) => Reply::Error(format!("Error writing file '{path}': {error}")),
            }
        })
    }

    fn edit(&self, path: &str, change: Replacement<'_>) -> impl Future<Output = EditResult> + Send {
        files::when_writable(self, files::edit(self, path, change))
    }

    fn delete(&self, path: &str) -> impl Future<Output = DeleteResult> + Send {
        files::when_writable(self, files::delete(self, path))
    }

    fn glob(&self, pattern: &str, path: &str) -> impl Future<Output = GlobResult> + Send {
        search::glob(self, pattern, path)
    }

    fn grep(&self, query: GrepQuery<'_>) -> impl Future<Output = GrepResult> + Send {
        search::grep(self, query)
    }

    fn upload_files(
        &self,
        files: &[(String, Vec<u8>)],
    ) -> impl Future<Output = Vec<FileUploadResponse>> + Send {
        stream::iter(files)
            .then(|file| transfer::upload(self, file))
            .collect()
    }

    fn download_files(
        &self,
        paths: &[String],
    ) -> impl Future<Output = Vec<FileDownloadResponse>> + Send {
        stream::iter(paths)
            .then(|path| files::download(self, path))
            .collect()
    }

    fn execute(
        &self,
        command: &str,
        timeout_seconds: Option<u64>,
    ) -> impl Future<Output = ExecuteResponse> + Send {
        execute::execute(self, command, timeout_seconds)
    }
}
