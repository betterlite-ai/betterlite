use std::time::Duration;

use betterlite_config::rag::document::ingest::VertexAIVectorStoreOptions;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use secrecy::SecretString;
use serde::Deserialize;

use crate::vertex_rag_ingest::chunking::RagChunkingStrategy;
use crate::vertex_rag_ingest::error::RagIngestError;

pub const DEFAULT_LOCATION: &str = "us-central1";
pub const DEFAULT_IMPORT_TIMEOUT: Duration = Duration::from_secs(600);

// The orchestrator's ingest options as this provider reads them: the rest of the document is not the provider's
#[derive(Debug, Default, Deserialize)]
pub struct RagIngestOptions {
    #[serde(default)]
    pub chunking_strategy: Option<RagChunkingStrategy>,
    #[serde(default)]
    pub vector_store: Option<VertexAIVectorStoreOptions>,
}

#[derive(Debug, Clone)]
pub struct RagIngestSettings {
    pub corpus_id: String,
    pub corpus_name: String,
    pub bucket: String,
    pub project: String,
    pub location: String,
    pub credentials: Option<SecretString>,
    pub wait_for_import: bool,
    pub import_timeout: Duration,
    pub chunking_strategy: Option<RagChunkingStrategy>,
}

impl RagIngestSettings {
    // LiteLLM checks the corpus, then the bucket, then the project; each value falls back the same way its `or` chain does
    pub fn resolve(
        options: &RagIngestOptions,
        vertex: &VertexSettings,
        bucket_environment: Option<&str>,
    ) -> Result<Self, RagIngestError> {
        let record = options.vector_store.as_ref();
        let corpus_id = record
            .and_then(|store| store.vector_store_id.as_deref())
            .filter(|text| !text.is_empty())
            .ok_or(RagIngestError::MissingCorpus)?;
        let bucket = record
            .and_then(|store| store.gcs_bucket.as_deref())
            .filter(|text| !text.is_empty())
            .or_else(|| bucket_environment.filter(|text| !text.is_empty()))
            .ok_or(RagIngestError::MissingBucket)?;
        let project = record
            .and_then(|store| store.vertex_project.as_deref())
            .filter(|text| !text.is_empty())
            .or_else(|| vertex.project.as_deref().filter(|text| !text.is_empty()))
            .ok_or(RagIngestError::MissingProject)?;
        let location = record
            .and_then(|store| store.vertex_location.as_deref())
            .filter(|text| !text.is_empty())
            .or_else(|| vertex.location.as_deref().filter(|text| !text.is_empty()))
            .unwrap_or(DEFAULT_LOCATION);
        Ok(Self {
            corpus_name: format!("projects/{project}/locations/{location}/ragCorpora/{corpus_id}"),
            corpus_id: corpus_id.to_owned(),
            bucket: bucket.to_owned(),
            project: project.to_owned(),
            location: location.to_owned(),
            credentials: record.and_then(|store| store.vertex_credentials.clone()),
            wait_for_import: record
                .and_then(|store| store.wait_for_import)
                .unwrap_or(true),
            // int() keeps every integer; a negative timeout is already expired, as Duration::ZERO is here
            import_timeout: record
                .and_then(|store| store.import_timeout)
                .map_or(DEFAULT_IMPORT_TIMEOUT, |seconds| {
                    Duration::from_secs(u64::try_from(seconds).unwrap_or_default())
                }),
            chunking_strategy: options.chunking_strategy.clone(),
        })
    }
}
