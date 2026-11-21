use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use tracing::{error, warn};

use super::dimension::{EmbedError, RagEmbedder, detect_dimension};
use super::settings::S3VectorsSettings;
use crate::s3_vectors_ingestion::{S3VectorData, S3VectorEntry, VectorIndexConfig};

pub mod signed;

pub const FALLBACK_EMBEDDING_MODEL: &str = "text-embedding-3-small";
pub const NO_CONTENT_MESSAGE: &str = concat!(
    "No text content could be extracted from the file for embedding. Possible causes:\n",
    "  1. PDF files require OCR - add 'ocr' config with a vision model (e.g., 'anthropic/claude-3-5-sonnet-20241022')\n",
    "  2. Binary files cannot be processed - convert to text first\n",
    "  3. File is empty or contains no extractable text\n",
    "For PDFs, either enable OCR or use a PDF extraction library to convert to text before ingestion.",
);

#[derive(Debug, thiserror::Error)]
pub enum WorkflowError<ApiFailure>
where
    ApiFailure: StdError,
{
    #[error(transparent)]
    Embed(#[from] EmbedError),
    #[error("{}", NO_CONTENT_MESSAGE)]
    NoContent,
    #[error(transparent)]
    Api(ApiFailure),
}

#[async_trait]
pub trait S3VectorsApi: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn ensure_bucket(&self, bucket_name: &str) -> Result<(), Self::Error>;

    async fn ensure_index(&self, index: &VectorIndexConfig<'_>) -> Result<(), Self::Error>;

    async fn put_vectors(
        &self,
        bucket_name: &str,
        index_name: &str,
        vectors: &[S3VectorEntry],
    ) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIndex {
    pub bucket_name: String,
    pub index_name: String,
}

#[derive(Debug)]
struct RuntimeSettings {
    dimension: Option<i64>,
    index_name: Option<String>,
    embedding_model: Option<String>,
    resolved: Option<ResolvedIndex>,
}

pub struct S3VectorsWorkflow<Api, Embedder> {
    settings: S3VectorsSettings,
    runtime: Mutex<RuntimeSettings>,
    api: Api,
    embedder: Embedder,
    unique_id: String,
}

impl<Api, Embedder> S3VectorsWorkflow<Api, Embedder>
where
    Api: S3VectorsApi,
    Embedder: RagEmbedder,
{
    #[must_use]
    pub fn new(
        settings: S3VectorsSettings,
        api: Api,
        embedder: Embedder,
        unique_id: String,
    ) -> Self {
        let runtime = RuntimeSettings {
            dimension: settings.dimension,
            index_name: settings.index_name.clone(),
            embedding_model: settings.embedding_model.clone(),
            resolved: None,
        };
        Self {
            settings,
            runtime: Mutex::new(runtime),
            api,
            embedder,
            unique_id,
        }
    }

    pub async fn ensure_config_initialized(
        &self,
    ) -> Result<ResolvedIndex, WorkflowError<Api::Error>> {
        let initialized = {
            let runtime = self.runtime.lock().unwrap_or_else(PoisonError::into_inner);
            runtime.resolved.clone()
        };
        if let Some(resolved) = initialized {
            return Ok(resolved);
        }
        let dimension = self.resolved_dimension().await;
        self.api
            .ensure_bucket(&self.settings.vector_bucket_name)
            .await
            .map_err(WorkflowError::Api)?;
        let index_name = self.resolved_index_name();
        let index = self.index_config(&index_name, dimension);
        self.api
            .ensure_index(&index)
            .await
            .map_err(WorkflowError::Api)?;
        let resolved = ResolvedIndex {
            bucket_name: self.settings.vector_bucket_name.clone(),
            index_name,
        };
        self.runtime
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .resolved = Some(resolved.clone());
        Ok(resolved)
    }

    pub async fn embed(
        &self,
        chunks: &[String],
    ) -> Result<Option<Vec<Vec<f64>>>, WorkflowError<Api::Error>> {
        if chunks.is_empty() {
            return Ok(None);
        }
        let configured = {
            let runtime = self.runtime.lock().unwrap_or_else(PoisonError::into_inner);
            runtime.embedding_model.clone()
        };
        let model = configured.unwrap_or_else(|| {
            warn!("No embedding config provided, using default text-embedding-3-small");
            let fallback = FALLBACK_EMBEDDING_MODEL.to_owned();
            self.runtime
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .embedding_model = Some(fallback.clone());
            fallback
        });
        let vectors = self
            .embedder
            .embed(&model, chunks.to_vec())
            .await
            .map_err(WorkflowError::Embed)?;
        Ok(Some(vectors))
    }

    // clippy's same_name_method refuses `store` while the type also implements RagStore
    pub async fn store_vectors(
        &self,
        filename: Option<&str>,
        chunks: &[String],
        embeddings: Option<&[Vec<f64>]>,
    ) -> Result<(Option<String>, Option<String>), WorkflowError<Api::Error>> {
        let resolved = self.ensure_config_initialized().await?;
        let vectors = embeddings.unwrap_or(&[]);
        if chunks.is_empty() || vectors.is_empty() {
            error!("{NO_CONTENT_MESSAGE}");
            return Err(WorkflowError::NoContent);
        }
        let entries = chunks
            .iter()
            .zip(vectors)
            .enumerate()
            .map(|(index, (chunk, vector))| vector_entry(filename, index, chunk, vector))
            .collect::<Vec<_>>();
        self.api
            .put_vectors(&resolved.bucket_name, &resolved.index_name, &entries)
            .await
            .map_err(WorkflowError::Api)?;
        let vector_store_id = format!("{}:{}", resolved.bucket_name, resolved.index_name);
        Ok((Some(vector_store_id), filename.map(str::to_owned)))
    }

    fn index_config<'name>(
        &'name self,
        index_name: &'name str,
        dimension: i64,
    ) -> VectorIndexConfig<'name> {
        VectorIndexConfig {
            bucket_name: &self.settings.vector_bucket_name,
            index_name,
            dimension,
            distance_metric: &self.settings.distance_metric,
            non_filterable_metadata_keys: &self.settings.non_filterable_metadata_keys,
        }
    }

    async fn resolved_dimension(&self) -> i64 {
        let (dimension, model) = {
            let runtime = self.runtime.lock().unwrap_or_else(PoisonError::into_inner);
            (runtime.dimension, runtime.embedding_model.clone())
        };
        if let Some(configured) = dimension {
            return configured;
        }
        let detected = detect_dimension(
            &self.embedder,
            model.as_deref(),
            self.settings.default_dimension,
        )
        .await;
        self.runtime
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .dimension = Some(detected);
        detected
    }

    fn resolved_index_name(&self) -> String {
        let configured = {
            let runtime = self.runtime.lock().unwrap_or_else(PoisonError::into_inner);
            runtime.index_name.clone()
        };
        if let Some(index_name) = configured {
            return index_name;
        }
        let generated = format!("litellm-index-{}", self.unique_id);
        self.runtime
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .index_name = Some(generated.clone());
        generated
    }
}

fn vector_entry(
    filename: Option<&str>,
    index: usize,
    chunk: &str,
    vector: &[f64],
) -> S3VectorEntry {
    let name = filename.filter(|value| !value.is_empty());
    let key = name.map_or_else(
        || format!("chunk_{index}"),
        |value| format!("{value}_{index}"),
    );
    let mut metadata = BTreeMap::from([
        ("source_text".to_owned(), chunk.to_owned()),
        ("chunk_index".to_owned(), index.to_string()),
    ]);
    if let Some(value) = name {
        metadata.insert("filename".to_owned(), value.to_owned());
    }
    S3VectorEntry {
        key,
        data: S3VectorData {
            float32: vector.to_vec(),
        },
        metadata,
    }
}
