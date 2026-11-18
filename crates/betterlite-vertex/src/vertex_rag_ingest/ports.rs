use std::future::Future;
use std::pin::Pin;

use betterlite_openai::files::records::{CreatePurpose, FileObject, FileUpload};
use secrecy::SecretString;
use thiserror::Error;

use crate::vertex_files::error::VertexFilesError;
use crate::vertex_rag_ingest::chunking::ChunkingRecord;

// litellm.files.acreate_file with custom_llm_provider="vertex_ai": the native upload itself is another task's
pub struct UploadRequest {
    pub file: FileUpload,
    pub purpose: CreatePurpose,
    pub bucket: String,
    pub project: String,
    pub location: String,
    pub credentials: Option<SecretString>,
}

pub type UploadFuture<'request> =
    Pin<Box<dyn Future<Output = Result<FileObject, VertexFilesError>> + Send + 'request>>;

pub trait NativeFileUpload: Send + Sync {
    fn upload<'request>(&'request self, request: UploadRequest) -> UploadFuture<'request>;
}

pub struct ImportRequest {
    pub corpus: String,
    pub gcs_uri: String,
    pub chunking: ChunkingRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportOutcome {
    pub imported_rag_files_count: i64,
}

pub type SubmitFuture<'request, Handle> =
    Pin<Box<dyn Future<Output = Result<Handle, ImportError>> + Send + 'request>>;

pub type ImportFuture<'request> =
    Pin<Box<dyn Future<Output = Result<ImportOutcome, ImportError>> + Send + 'request>>;

// vertexai.rag.import_files: the submission starts the operation, and only the wait after it takes the timeout
pub trait CorpusImport: Send + Sync {
    type Handle: Send + 'static;
    fn submit<'request>(
        &'request self,
        request: ImportRequest,
    ) -> SubmitFuture<'request, Self::Handle>;
    fn wait<'request>(&'request self, handle: Self::Handle) -> ImportFuture<'request>;
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("the Vertex RAG client could not be created: {source}")]
    Client {
        #[source]
        source: google_cloud_gax::client_builder::Error,
    },
    #[error("the import operation failed: {source}")]
    Rpc {
        #[source]
        source: google_cloud_gax::error::Error,
    },
    #[error("the import submission timed out after {seconds} seconds")]
    SubmitTimeout { seconds: u64 },
    #[error("the import timed out after {seconds} seconds")]
    Timeout { seconds: u64 },
    #[error("the {field} value {value} does not fit the Vertex RAG integer field")]
    IntegerRange { field: &'static str, value: i64 },
}
