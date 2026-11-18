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
    #[error("the file is {size} bytes, over the {limit} byte Vertex RAG ingestion limit")]
    FileTooLarge { size: usize, limit: usize },
    #[error("the filename is {length} bytes, over the {limit} byte filename limit")]
    FilenameTooLong { length: usize, limit: usize },
    #[error("the content type is {length} bytes, over the {limit} byte content type limit")]
    ContentTypeTooLong { length: usize, limit: usize },
    #[error("the file upload timed out after {seconds} seconds")]
    UploadTimeout { seconds: u64 },
    #[error(transparent)]
    Upload(#[from] VertexFilesError),
}
