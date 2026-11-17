use thiserror::Error;

use crate::vertex_files::error::VertexFilesError;
use crate::vertex_rag_ingest::ports::ImportError;

#[derive(Debug, Error)]
pub enum RagIngestError {
    #[error(
        "vector_store_id (corpus ID) is required for Vertex AI RAG ingestion. Please provide an existing RAG corpus ID."
    )]
    MissingCorpus,
    #[error(
        "gcs_bucket is required for Vertex AI RAG ingestion. Set via vector_store config or GCS_BUCKET_NAME env var."
    )]
    MissingBucket,
    #[error(
        "vertex_project is required for Vertex AI RAG ingestion. Set via vector_store config or VERTEXAI_PROJECT env var."
    )]
    MissingProject,
    #[error("Failed to import file into RAG corpus: {source}")]
    Import {
        #[source]
        source: ImportError,
    },
    #[error(transparent)]
    Upload(#[from] VertexFilesError),
}
