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
        let non_empty = |value: Option<&str>| value.filter(|text| !text.is_empty());
        let corpus_id = non_empty(record.and_then(|record| record.vector_store_id.as_deref()))
            .ok_or(RagIngestError::MissingCorpus)?;
        let bucket = non_empty(record.and_then(|record| record.gcs_bucket.as_deref()))
            .or_else(|| non_empty(bucket_environment))
            .ok_or(RagIngestError::MissingBucket)?;
        let project = non_empty(record.and_then(|record| record.vertex_project.as_deref()))
            .or_else(|| non_empty(vertex.project.as_deref()))
            .ok_or(RagIngestError::MissingProject)?;
        let location = non_empty(record.and_then(|record| record.vertex_location.as_deref()))
            .or_else(|| non_empty(vertex.location.as_deref()))
            .unwrap_or(DEFAULT_LOCATION);
        Ok(Self {
            corpus_name: format!("projects/{project}/locations/{location}/ragCorpora/{corpus_id}"),
            corpus_id: corpus_id.to_owned(),
            bucket: bucket.to_owned(),
            project: project.to_owned(),
            location: location.to_owned(),
            credentials: record.and_then(|record| record.vertex_credentials.clone()),
            wait_for_import: record
                .and_then(|record| record.wait_for_import)
                .unwrap_or(true),
            // int() keeps every integer; a negative timeout is already expired, as Duration::ZERO is here
            import_timeout: record
                .and_then(|record| record.import_timeout)
                .map_or(DEFAULT_IMPORT_TIMEOUT, |seconds| {
                    Duration::from_secs(u64::try_from(seconds).unwrap_or_default())
                }),
            chunking_strategy: options.chunking_strategy.clone(),
        })
    }
}
