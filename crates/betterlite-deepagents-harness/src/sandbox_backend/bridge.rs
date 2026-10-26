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
    fn als(&self, path: &str) -> impl Future<Output = LsResult> + Send;
    fn aread(
        &self,
        path: &str,
        offset: usize,
        limit: usize,
    ) -> impl Future<Output = ReadResult> + Send;
    fn awrite(&self, path: &str, content: &str) -> impl Future<Output = WriteResult> + Send;
    fn aedit(&self, path: &str, change: Replacement<'_>)
    -> impl Future<Output = EditResult> + Send;
    fn adelete(&self, path: &str) -> impl Future<Output = DeleteResult> + Send;
    fn aglob(&self, pattern: &str, path: &str) -> impl Future<Output = GlobResult> + Send;
    fn agrep(&self, query: GrepQuery<'_>) -> impl Future<Output = GrepResult> + Send;
    fn aupload_files(
        &self,
        files: &[(String, Vec<u8>)],
    ) -> impl Future<Output = Vec<FileUploadResponse>> + Send;
    fn adownload_files(
        &self,
        paths: &[String],
    ) -> impl Future<Output = Vec<FileDownloadResponse>> + Send;
    fn aexecute(
        &self,
        command: &str,
        timeout_seconds: Option<u64>,
    ) -> impl Future<Output = ExecuteResponse> + Send;

    fn ls(&self, path: &str) -> Result<LsResult, BackendError> {
        self.block_on(self.als(path))
    }

    fn read(&self, path: &str, offset: usize, limit: usize) -> Result<ReadResult, BackendError> {
        self.block_on(self.aread(path, offset, limit))
    }

    fn write(&self, path: &str, content: &str) -> Result<WriteResult, BackendError> {
        self.block_on(self.awrite(path, content))
    }

    fn edit(&self, path: &str, change: Replacement<'_>) -> Result<EditResult, BackendError> {
        self.block_on(self.aedit(path, change))
    }

    fn delete(&self, path: &str) -> Result<DeleteResult, BackendError> {
        self.block_on(self.adelete(path))
    }

    fn glob(&self, pattern: &str, path: &str) -> Result<GlobResult, BackendError> {
        self.block_on(self.aglob(pattern, path))
    }

    fn grep(&self, query: GrepQuery<'_>) -> Result<GrepResult, BackendError> {
        self.block_on(self.agrep(query))
    }

    fn upload_files(
        &self,
        files: &[(String, Vec<u8>)],
    ) -> Result<Vec<FileUploadResponse>, BackendError> {
        self.block_on(self.aupload_files(files))
    }

    fn download_files(&self, paths: &[String]) -> Result<Vec<FileDownloadResponse>, BackendError> {
        self.block_on(self.adownload_files(paths))
    }

    fn execute(
        &self,
        command: &str,
        timeout_seconds: Option<u64>,
    ) -> Result<ExecuteResponse, BackendError> {
        self.block_on(self.aexecute(command, timeout_seconds))
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

    fn als(&self, path: &str) -> impl Future<Output = LsResult> + Send {
        files::list(self, path)
    }

    fn aread(
        &self,
        path: &str,
        offset: usize,
        limit: usize,
    ) -> impl Future<Output = ReadResult> + Send {
        files::read(self, path, offset, limit)
    }

    fn awrite(&self, path: &str, content: &str) -> impl Future<Output = WriteResult> + Send {
        files::when_writable(self, async move {
            match self.write_bytes(path, content.as_bytes()).await {
                Ok(()) => Reply::Done(path.to_owned()),
                Err(error) => Reply::Error(format!("Error writing file '{path}': {error}")),
            }
        })
    }

    fn aedit(
        &self,
        path: &str,
        change: Replacement<'_>,
    ) -> impl Future<Output = EditResult> + Send {
        files::when_writable(self, files::edit(self, path, change))
    }

    fn adelete(&self, path: &str) -> impl Future<Output = DeleteResult> + Send {
        files::when_writable(self, files::delete(self, path))
    }

    fn aglob(&self, pattern: &str, path: &str) -> impl Future<Output = GlobResult> + Send {
        search::glob(self, pattern, path)
    }

    fn agrep(&self, query: GrepQuery<'_>) -> impl Future<Output = GrepResult> + Send {
        search::grep(self, query)
    }

    fn aupload_files(
        &self,
        files: &[(String, Vec<u8>)],
    ) -> impl Future<Output = Vec<FileUploadResponse>> + Send {
        stream::iter(files)
            .then(|file| transfer::upload(self, file))
            .collect()
    }

    fn adownload_files(
        &self,
        paths: &[String],
    ) -> impl Future<Output = Vec<FileDownloadResponse>> + Send {
        stream::iter(paths)
            .then(|path| files::download(self, path))
            .collect()
    }

    fn aexecute(
        &self,
        command: &str,
        timeout_seconds: Option<u64>,
    ) -> impl Future<Output = ExecuteResponse> + Send {
        execute::execute(self, command, timeout_seconds)
    }
}
