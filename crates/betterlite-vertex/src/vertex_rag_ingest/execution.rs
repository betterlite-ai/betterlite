use std::future::{Future, ready};
use std::pin::Pin;

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
    upload: Upload,
    import: Import,
}

impl<Upload: NativeFileUpload, Import: CorpusImport> VertexRagIngest<Upload, Import> {
    #[must_use]
    pub const fn new(
        settings: RagIngestSettings,
        chunk_defaults: ChunkDefaults,
        upload: Upload,
        import: Import,
    ) -> Self {
        Self {
            settings,
            chunk_defaults,
            upload,
            import,
        }
    }

    async fn store_bytes(&self, request: StoreRequest<'_>) -> Result<StoreOutcome, RagIngestError> {
        let content = request.file_content.filter(|content| !content.is_empty());
        let filename = request.filename.filter(|filename| !filename.is_empty());
        let (Some(content), Some(filename)) = (content, filename) else {
            tracing::warn!("No file content or filename provided for Vertex AI ingestion");
            return Ok(StoreOutcome {
                corpus_id: Some(self.settings.corpus_id.clone()),
                file_id: None,
            });
        };
        let content_type = request
            .content_type
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_CONTENT_TYPE);
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
        let uploaded = self
            .upload
            .upload(UploadRequest {
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
            })
            .await?;
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
        let attempt = timeout(
            self.settings.import_timeout,
            self.import.import(ImportRequest {
                corpus: self.settings.corpus_name.clone(),
                gcs_uri: gcs_uri.to_owned(),
                chunking,
            }),
        )
        .await
        .map_err(|_elapsed| ImportError::Timeout {
            seconds: self.settings.import_timeout.as_secs(),
        });
        match attempt {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(source)) | Err(source) => {
                tracing::error!("Failed to import file into RAG corpus: {source}");
                Err(RagIngestError::Import { source })
            }
        }
    }
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
