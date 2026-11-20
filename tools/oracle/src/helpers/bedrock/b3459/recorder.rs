use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_rag::s3_vectors_ingestion::{S3VectorEntry, VectorIndexConfig};
use betterlite_rag::s3_vectors_workflow::dimension::{EmbedError, RagEmbedder};
use betterlite_rag::s3_vectors_workflow::workflow::S3VectorsApi;
use serde_json::{Value, json};

#[derive(Debug, Clone, Default)]
pub(crate) struct Recorder {
    operations: Arc<Mutex<Vec<Value>>>,
}

impl Recorder {
    pub(crate) fn record(&self, operation: Value) {
        self.operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(operation);
    }

    pub(crate) fn operations(&self) -> Value {
        let held = self
            .operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Value::Array(held.clone())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted S3 Vectors API is never called in this step")]
pub(crate) struct ScriptedApiError;

#[derive(Debug, Clone)]
pub(crate) struct RecordingApi {
    pub(crate) recorder: Recorder,
}

#[async_trait]
impl S3VectorsApi for RecordingApi {
    type Error = ScriptedApiError;

    async fn ensure_bucket(&self, bucket_name: &str) -> Result<(), Self::Error> {
        self.recorder
            .record(json!({"call": "ensure_bucket", "bucket": bucket_name}));
        Ok(())
    }

    async fn ensure_index(&self, index: &VectorIndexConfig<'_>) -> Result<(), Self::Error> {
        self.recorder.record(json!({
            "call": "ensure_index",
            "bucket": index.bucket_name,
            "index": index.index_name,
            "dimension": index.dimension,
            "distance_metric": index.distance_metric,
            "non_filterable_metadata_keys": index.non_filterable_metadata_keys,
        }));
        Ok(())
    }

    async fn put_vectors(
        &self,
        bucket_name: &str,
        index_name: &str,
        vectors: &[S3VectorEntry],
    ) -> Result<(), Self::Error> {
        self.recorder.record(json!({
            "call": "put_vectors",
            "bucket": bucket_name,
            "index": index_name,
            "vectors": vectors,
        }));
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ScriptedEmbedder {
    recorder: Recorder,
    answer: Vec<Vec<f64>>,
    failing: bool,
}

impl ScriptedEmbedder {
    pub(crate) fn new(recorder: Recorder, answer: Vec<Vec<f64>>, failing: bool) -> Self {
        Self {
            recorder,
            answer,
            failing,
        }
    }
}

#[async_trait]
impl RagEmbedder for ScriptedEmbedder {
    async fn embed(&self, model: &str, input: Vec<String>) -> Result<Vec<Vec<f64>>, EmbedError> {
        self.recorder
            .record(json!({"call": "embedding", "model": model, "input": input}));
        if self.failing {
            return Err(EmbedError::Failed {
                model: model.to_owned(),
                message: "scripted embedding failure".to_owned(),
            });
        }
        Ok(self.answer.clone())
    }
}
