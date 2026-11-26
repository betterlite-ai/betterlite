use betterlite_vertex::vertex_batch_records::error::BatchRecordsError;
use thiserror::Error;
use tokio::runtime::TryCurrentError;

#[derive(Debug, Error)]
pub(crate) enum VertexBatchError {
    #[error("the batch prediction job response body is not valid JSON: {source}")]
    Parse {
        #[source]
        source: serde_json::Error,
    },
    #[error("the batch prediction job response is not a Vertex AI batch job: {source}")]
    Job {
        #[source]
        source: serde_json::Error,
    },
    #[error("the batch prediction job response cannot become a batch record: {source}")]
    Transform {
        #[source]
        source: BatchRecordsError,
    },
    #[error("the batch prediction job model is {kind}, not a string")]
    ModelNotText { kind: &'static str },
    #[error("the batch managed object cannot be spawned without a Tokio runtime: {source}")]
    NoRuntime {
        #[source]
        source: TryCurrentError,
    },
}
