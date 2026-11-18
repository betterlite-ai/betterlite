use std::future::{Future, ready};
use std::pin::Pin;
use std::time::Duration;

use betterlite_core::rag::upload_security::MAX_UPLOAD_SIZE_BYTES;
use betterlite_openai::files::records::{CreatePurpose, FileUpload};
use tokio::time::timeout;

use crate::vertex_rag_ingest::chunking::{
    ChunkDefaults, ChunkingRecord, transform_chunking_strategy,
};
use crate::vertex_rag_ingest::error::RagIngestError;
use crate::vertex_rag_ingest::options::RagIngestSettings;
use crate::vertex_rag_ingest::ports::{
    CorpusImport, ImportError, ImportOutcome, ImportRequest, NativeFileUpload, UploadRequest,
};

pub const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";
// LiteLLM's acreate_file call passes no timeout, and its files layer defaults that to ten minutes
pub const DEFAULT_UPLOAD_TIMEOUT: Duration = Duration::from_secs(600);
// google-api-core bounds the import submission RPC with its 60 second default request timeout
pub const DEFAULT_IMPORT_SUBMIT_TIMEOUT: Duration = Duration::from_secs(60);
// the GCS object name holds 1024 bytes, and the filename becomes that name
pub const MAX_FILENAME_BYTES: usize = 1024;
pub const MAX_CONTENT_TYPE_BYTES: usize = 255;

// store refuses inputs over these bounds before it copies them into an upload
#[derive(Debug, Clone, Copy)]
pub struct StoreLimits {
    pub max_file_bytes: usize,
    pub max_filename_bytes: usize,
    pub max_content_type_bytes: usize,
}

impl Default for StoreLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: MAX_UPLOAD_SIZE_BYTES,
            max_filename_bytes: MAX_FILENAME_BYTES,
            max_content_type_bytes: MAX_CONTENT_TYPE_BYTES,
        }
    }
}

impl StoreLimits {
    // betterlite's shared 512 MiB file cap bounds the copy, and LiteLLM has no size check to follow
    const fn check(
        &self,
        content: &[u8],
        filename: &str,
        content_type: &str,
    ) -> Result<(), RagIngestError> {
        if content.len() > self.max_file_bytes {
            return Err(RagIngestError::FileTooLarge {
                size: content.len(),
                limit: self.max_file_bytes,
            });
        }
        if filename.len() > self.max_filename_bytes {
            return Err(RagIngestError::FilenameTooLong {
                length: filename.len(),
                limit: self.max_filename_bytes,
            });
        }
        if content_type.len() > self.max_content_type_bytes {
            return Err(RagIngestError::ContentTypeTooLong {
                length: content_type.len(),
                limit: self.max_content_type_bytes,
            });
        }
        Ok(())
    }
}

// LiteLLM's (corpus_id, gcs_uri) return: both sides stay optional because an empty source stores nothing
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreOutcome {
    pub corpus_id: Option<String>,
    pub file_id: Option<String>,
}

// LiteLLM's store arguments: chunks, embeddings and an existing file id never replace the byte upload
#[derive(Default)]
pub struct StoreRequest<'request> {
    pub file_content: Option<&'request [u8]>,
    pub filename: Option<&'request str>,
    pub content_type: Option<&'request str>,
    pub chunks: &'request [String],
    pub embeddings: Option<&'request [Vec<f32>]>,
    pub existing_file_id: Option<&'request str>,
}

pub type EmbedFuture<'request> =
    Pin<Box<dyn Future<Output = Option<Vec<Vec<f32>>>> + Send + 'request>>;
pub type StoreFuture<'request> =
    Pin<Box<dyn Future<Output = Result<StoreOutcome, RagIngestError>> + Send + 'request>>;

// The provider contract the RAG orchestrator calls: Vertex embeds and chunks the file inside the corpus
pub trait RagIngest: Send + Sync {
    fn embed<'request>(&'request self, chunks: &'request [String]) -> EmbedFuture<'request>;
    fn store<'request>(&'request self, request: StoreRequest<'request>) -> StoreFuture<'request>;
}

pub struct VertexRagIngest<Upload, Import> {
    settings: RagIngestSettings,
    chunk_defaults: ChunkDefaults,
    limits: StoreLimits,
    upload: Upload,
    import: Import,
}

impl<Upload: NativeFileUpload, Import: CorpusImport> VertexRagIngest<Upload, Import> {
    #[must_use]
    pub const fn new(
        settings: RagIngestSettings,
        chunk_defaults: ChunkDefaults,
        limits: StoreLimits,
        upload: Upload,
        import: Import,
    ) -> Self {
        Self {
            settings,
            chunk_defaults,
            limits,
            upload,
            import,
        }
    }

    async fn store_bytes(&self, request: StoreRequest<'_>) -> Result<StoreOutcome, RagIngestError> {
        let StoreRequest {
            file_content,
            filename: object,
            content_type: declared_type,
            ..
        } = request;
        let (Some(content), Some(filename)) = (
            file_content.filter(|data| !data.is_empty()),
            object.filter(|text| !text.is_empty()),
        ) else {
            tracing::warn!("No file content or filename provided for Vertex AI ingestion");
            return Ok(StoreOutcome {
                corpus_id: Some(self.settings.corpus_id.clone()),
                file_id: None,
            });
        };
        let content_type = declared_type
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_CONTENT_TYPE);
        self.limits.check(content, filename, content_type)?;
        let gcs_uri = self.upload_bytes(content, filename, content_type).await?;
        self.import_file(&gcs_uri).await?;
        Ok(StoreOutcome {
            corpus_id: Some(self.settings.corpus_id.clone()),
            file_id: Some(gcs_uri),
        })
    }

    async fn upload_bytes(
        &self,
        content: &[u8],
        filename: &str,
        content_type: &str,
    ) -> Result<String, RagIngestError> {
        tracing::debug!(
            "Uploading file to GCS via litellm.files.acreate_file: {filename} (bucket: {})",
            self.settings.bucket
        );
        let request = UploadRequest {
            file: FileUpload {
                filename: Some(filename.to_owned()),
                content_type: Some(content_type.to_owned()),
                data: content.to_vec(),
            },
            purpose: CreatePurpose::Assistants,
            bucket: self.settings.bucket.clone(),
            project: self.settings.project.clone(),
            location: self.settings.location.clone(),
            credentials: self.settings.credentials.clone(),
        };
        let uploaded = timeout(DEFAULT_UPLOAD_TIMEOUT, self.upload.upload(request))
            .await
            .map_err(|_elapsed| RagIngestError::UploadTimeout {
                seconds: DEFAULT_UPLOAD_TIMEOUT.as_secs(),
            })??;
        tracing::info!("Uploaded file to GCS: {}", uploaded.id);
        Ok(uploaded.id)
    }

    async fn import_file(&self, gcs_uri: &str) -> Result<(), RagIngestError> {
        // LiteLLM builds the transformation before its wait branch, so ignored separators warn even without a wait
        let chunking = transform_chunking_strategy(
            self.settings.chunking_strategy.as_ref(),
            self.chunk_defaults,
        );
        tracing::debug!(
            "Importing {gcs_uri} into corpus {}",
            self.settings.corpus_id
        );
        if !self.settings.wait_for_import {
            // LiteLLM calls the SDK's async import without awaiting it, so no import request leaves the process
            tracing::info!("Import started asynchronously");
            return Ok(());
        }
        let outcome = self.submit_import(chunking, gcs_uri).await?;
        tracing::info!(
            "Import complete: {} files imported",
            outcome.imported_rag_files_count
        );
        Ok(())
    }

    async fn submit_import(
        &self,
        chunking: ChunkingRecord,
        gcs_uri: &str,
    ) -> Result<ImportOutcome, RagIngestError> {
        // import_timeout covers the wait LiteLLM does after the SDK submits the operation, not the submission
        let submitted = timeout(
            DEFAULT_IMPORT_SUBMIT_TIMEOUT,
            self.import.submit(ImportRequest {
                corpus: self.settings.corpus_name.clone(),
                gcs_uri: gcs_uri.to_owned(),
                chunking,
            }),
        )
        .await
        .map_err(|_elapsed| ImportError::SubmitTimeout {
            seconds: DEFAULT_IMPORT_SUBMIT_TIMEOUT.as_secs(),
        });
        let handle = match submitted {
            Ok(Ok(handle)) => handle,
            Ok(Err(source)) | Err(source) => return Err(import_failure(source)),
        };
        let waited = timeout(self.settings.import_timeout, self.import.wait(handle))
            .await
            .map_err(|_elapsed| ImportError::Timeout {
                seconds: self.settings.import_timeout.as_secs(),
            });
        match waited {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(source)) | Err(source) => Err(import_failure(source)),
        }
    }
}

fn import_failure(source: ImportError) -> RagIngestError {
    tracing::error!("Failed to import file into RAG corpus: {source}");
    RagIngestError::Import { source }
}

impl<Upload: NativeFileUpload, Import: CorpusImport> RagIngest for VertexRagIngest<Upload, Import> {
    fn embed<'request>(&'request self, _chunks: &'request [String]) -> EmbedFuture<'request> {
        Box::pin(ready(None::<Vec<Vec<f32>>>))
    }

    fn store<'request>(&'request self, request: StoreRequest<'request>) -> StoreFuture<'request> {
        Box::pin(self.store_bytes(request))
    }
}

mod tests;
