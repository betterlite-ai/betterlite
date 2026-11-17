use std::future::{pending, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::rag::document::ingest::VertexAIVectorStoreOptions;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_openai::files::records::{
    CreatePurpose, FileKind, FileObject, FileStatus, FilesPurpose,
};
use google_cloud_gax::error::Error;
use secrecy::ExposeSecret as _;

use crate::vertex_rag_ingest::chunking::{ChunkDefaults, ChunkingRecord, RagChunkingStrategy};
use crate::vertex_rag_ingest::error::RagIngestError;
use crate::vertex_rag_ingest::execution::{StoreRequest, VertexRagIngest};
use crate::vertex_rag_ingest::options::{RagIngestOptions, RagIngestSettings};
use crate::vertex_rag_ingest::ports::{
    CorpusImport, ImportError, ImportFuture, ImportOutcome, ImportRequest, NativeFileUpload,
    UploadFuture, UploadRequest,
};

pub(super) const CORPUS_ID: &str = "corpus";
pub(super) const BUCKET: &str = "bucket";
pub(super) const PROJECT: &str = "project";
pub(super) const CORPUS_NAME: &str = "projects/project/locations/us-central1/ragCorpora/corpus";
pub(super) const GCS_URI: &str = "gs://bucket/report.txt";
pub(super) const EMPTY_CONTENT_WARNING: &str =
    "No file content or filename provided for Vertex AI ingestion";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    Upload,
    Import,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ImportBehavior {
    Imported(i64),
    Rejected,
    NeverFinishes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Uploaded {
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
    pub purpose: CreatePurpose,
    pub bucket: String,
    pub project: String,
    pub location: String,
    pub credentials: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Imported {
    pub corpus: String,
    pub gcs_uri: String,
    pub chunking: ChunkingRecord,
}

#[derive(Default)]
pub(super) struct Recorder {
    steps: Mutex<Vec<Step>>,
    uploads: Mutex<Vec<Uploaded>>,
    imports: Mutex<Vec<Imported>>,
    retained: Mutex<Vec<String>>,
}

impl Recorder {
    pub(super) fn steps(&self) -> Vec<Step> {
        let steps = self.steps.lock().unwrap_or_else(PoisonError::into_inner);
        steps.clone()
    }

    pub(super) fn upload(&self) -> Uploaded {
        let uploads = self.uploads.lock().unwrap_or_else(PoisonError::into_inner);
        uploads.first().expect("an upload happened").clone()
    }

    pub(super) fn imported(&self) -> Imported {
        let imports = self.imports.lock().unwrap_or_else(PoisonError::into_inner);
        imports.first().expect("an import happened").clone()
    }

    pub(super) fn retained(&self) -> Vec<String> {
        let retained = self.retained.lock().unwrap_or_else(PoisonError::into_inner);
        retained.clone()
    }
}

pub(super) struct FakeUpload {
    recorder: Arc<Recorder>,
}

impl NativeFileUpload for FakeUpload {
    fn upload<'request>(&'request self, request: UploadRequest) -> UploadFuture<'request> {
        let filename = request.file.filename.clone().unwrap_or_default();
        let uri = format!("gs://{}/{filename}", request.bucket);
        self.recorder
            .steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Step::Upload);
        self.recorder
            .uploads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Uploaded {
                filename: request.file.filename.clone(),
                content_type: request.file.content_type.clone(),
                data: request.file.data.clone(),
                purpose: request.purpose,
                bucket: request.bucket.clone(),
                project: request.project.clone(),
                location: request.location.clone(),
                credentials: request
                    .credentials
                    .as_ref()
                    .map(|secret| secret.expose_secret().to_owned()),
            });
        self.recorder
            .retained
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(uri.clone());
        let object = FileObject {
            id: uri,
            bytes: i64::try_from(request.file.data.len()).unwrap_or(i64::MAX),
            created_at: 0,
            filename,
            object: FileKind::File,
            purpose: FilesPurpose::Assistants,
            status: Some(FileStatus::Uploaded),
            expires_at: None,
            status_details: None,
        };
        Box::pin(ready(Ok(object)))
    }
}

pub(super) struct FakeImport {
    recorder: Arc<Recorder>,
    behavior: ImportBehavior,
}

impl CorpusImport for FakeImport {
    fn import<'request>(&'request self, request: ImportRequest) -> ImportFuture<'request> {
        self.recorder
            .steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Step::Import);
        self.recorder
            .imports
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Imported {
                corpus: request.corpus.clone(),
                gcs_uri: request.gcs_uri.clone(),
                chunking: request.chunking,
            });
        let behavior = self.behavior;
        Box::pin(async move {
            match behavior {
                ImportBehavior::Imported(count) => Ok(ImportOutcome {
                    imported_rag_files_count: count,
                }),
                ImportBehavior::Rejected => Err(ImportError::Rpc {
                    source: Error::deser("the corpus rejected the file"),
                }),
                ImportBehavior::NeverFinishes => pending().await,
            }
        })
    }
}

pub(super) fn vertex_settings(project: Option<&str>, location: Option<&str>) -> VertexSettings {
    VertexSettings {
        project: project.map(str::to_owned),
        location: location.map(str::to_owned),
        credentials: None,
        adc: AdcEnvironment::default(),
        default_video_duration_seconds: 8,
    }
}

// The record names the corpus and nothing else; a test sets the other fields it reads on top
pub(super) fn corpus_record() -> VertexAIVectorStoreOptions {
    VertexAIVectorStoreOptions {
        custom_llm_provider: None,
        vector_store_id: Some(CORPUS_ID.to_owned()),
        vertex_project: None,
        vertex_location: None,
        vertex_credentials: None,
        gcs_bucket: None,
        wait_for_import: None,
        import_timeout: None,
        description: None,
        embedding_model: None,
        vertex_ai_project: None,
        vertex_ai_location: None,
        vertex_ai_credentials: None,
        file_description: None,
        max_embedding_requests_per_min: None,
        litellm_credential_name: None,
        vector_db_config: None,
    }
}

pub(super) fn options(
    record: VertexAIVectorStoreOptions,
    chunking_strategy: Option<RagChunkingStrategy>,
) -> RagIngestOptions {
    RagIngestOptions {
        chunking_strategy,
        vector_store: Some(record),
    }
}

pub(super) fn resolve(options: &RagIngestOptions) -> Result<RagIngestSettings, RagIngestError> {
    RagIngestSettings::resolve(options, &vertex_settings(Some(PROJECT), None), Some(BUCKET))
}

pub(super) fn provider(
    record: VertexAIVectorStoreOptions,
    chunking_strategy: Option<RagChunkingStrategy>,
    behavior: ImportBehavior,
) -> (VertexRagIngest<FakeUpload, FakeImport>, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let settings =
        resolve(&options(record, chunking_strategy)).expect("the required values resolve");
    let ingest = VertexRagIngest::new(
        settings,
        ChunkDefaults::LITELLM,
        FakeUpload {
            recorder: Arc::clone(&recorder),
        },
        FakeImport {
            recorder: Arc::clone(&recorder),
            behavior,
        },
    );
    (ingest, recorder)
}

pub(super) fn store_request<'request>(
    content: &'request [u8],
    filename: &'request str,
    content_type: Option<&'request str>,
) -> StoreRequest<'request> {
    StoreRequest {
        file_content: Some(content),
        filename: Some(filename),
        content_type,
        chunks: &[],
        embeddings: None,
        existing_file_id: None,
    }
}
