use async_trait::async_trait;
use betterlite_core::rag::ingestion::store::{RagStore, StoreRequest, StoreResult};

use super::dimension::RagEmbedder;
use super::workflow::{S3VectorsApi, S3VectorsWorkflow, WorkflowError};

#[async_trait]
impl<Api, Embedder> RagStore for S3VectorsWorkflow<Api, Embedder>
where
    Api: S3VectorsApi,
    Embedder: RagEmbedder,
{
    type Error = WorkflowError<Api::Error>;

    async fn store(&self, request: StoreRequest<'_>) -> Result<StoreResult, Self::Error> {
        S3VectorsWorkflow::store(self, request.filename, request.chunks, request.embeddings).await
    }
}
