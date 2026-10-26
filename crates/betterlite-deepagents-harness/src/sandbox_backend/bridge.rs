use std::future::Future;

use betterlite_harness::sandbox::Sandbox;
use tokio::runtime::Handle;

use crate::sandbox_backend::errors::BackendError;
use crate::sandbox_backend::protocol::{
    DeleteResult, EditResult, ExecuteResponse, FileDownloadResponse, FileUploadResponse,
    GlobResult, GrepResult, LsResult, ReadResult, WriteResult,
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
        file_path: &str,
        offset: usize,
        limit: usize,
    ) -> impl Future<Output = ReadResult> + Send;
    fn awrite(&self, file_path: &str, content: &str) -> impl Future<Output = WriteResult> + Send;
    fn aedit(
        &self,
        file_path: &str,
        old: &str,
        new: &str,
        replace_all: bool,
    ) -> impl Future<Output = EditResult> + Send;
    fn adelete(&self, file_path: &str) -> impl Future<Output = DeleteResult> + Send;
    fn aglob(&self, pattern: &str, path: &str) -> impl Future<Output = GlobResult> + Send;
    fn agrep(
        &self,
        pattern: &str,
        path: &str,
        glob: Option<&str>,
        max_count: Option<usize>,
    ) -> impl Future<Output = GrepResult> + Send;
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

    fn read(
        &self,
        file_path: &str,
        offset: usize,
        limit: usize,
    ) -> Result<ReadResult, BackendError> {
        self.block_on(self.aread(file_path, offset, limit))
    }

    fn write(&self, file_path: &str, content: &str) -> Result<WriteResult, BackendError> {
        self.block_on(self.awrite(file_path, content))
    }

    fn edit(
        &self,
        file_path: &str,
        old: &str,
        new: &str,
        replace_all: bool,
    ) -> Result<EditResult, BackendError> {
        self.block_on(self.aedit(file_path, old, new, replace_all))
    }

    fn delete(&self, file_path: &str) -> Result<DeleteResult, BackendError> {
        self.block_on(self.adelete(file_path))
    }

    fn glob(&self, pattern: &str, path: &str) -> Result<GlobResult, BackendError> {
        self.block_on(self.aglob(pattern, path))
    }

    fn grep(
        &self,
        pattern: &str,
        path: &str,
        glob: Option<&str>,
        max_count: Option<usize>,
    ) -> Result<GrepResult, BackendError> {
        self.block_on(self.agrep(pattern, path, glob, max_count))
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
        file_path: &str,
        offset: usize,
        limit: usize,
    ) -> impl Future<Output = ReadResult> + Send {
        files::read(self, file_path, offset, limit)
    }

    fn awrite(&self, file_path: &str, content: &str) -> impl Future<Output = WriteResult> + Send {
        files::write(self, file_path, content)
    }

    fn aedit(
        &self,
        file_path: &str,
        old: &str,
        new: &str,
        replace_all: bool,
    ) -> impl Future<Output = EditResult> + Send {
        files::edit(self, file_path, old, new, replace_all)
    }

    fn adelete(&self, file_path: &str) -> impl Future<Output = DeleteResult> + Send {
        files::delete(self, file_path)
    }

    fn aglob(&self, pattern: &str, path: &str) -> impl Future<Output = GlobResult> + Send {
        search::glob(self, pattern, path)
    }

    fn agrep(
        &self,
        pattern: &str,
        path: &str,
        glob: Option<&str>,
        max_count: Option<usize>,
    ) -> impl Future<Output = GrepResult> + Send {
        search::grep(self, pattern, path, glob, max_count)
    }

    fn aupload_files(
        &self,
        files: &[(String, Vec<u8>)],
    ) -> impl Future<Output = Vec<FileUploadResponse>> + Send {
        transfer::upload(self, files)
    }

    fn adownload_files(
        &self,
        paths: &[String],
    ) -> impl Future<Output = Vec<FileDownloadResponse>> + Send {
        transfer::download(self, paths)
    }

    fn aexecute(
        &self,
        command: &str,
        timeout_seconds: Option<u64>,
    ) -> impl Future<Output = ExecuteResponse> + Send {
        execute::execute(self, command, timeout_seconds)
    }
}
