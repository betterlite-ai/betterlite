use super::error::SemanticEmbeddingError;
use super::{
    ConfiguredTokenLimits, EmbeddingCalls, EmbeddingMode, MetadataMap, SemanticEmbeddingService,
};

// ast-grep-ignore: no-padded-one-line-functions
pub async fn qdrant_embedding<R, C>(
    service: &SemanticEmbeddingService<R, C>,
    prompt: &str,
    metadata: Option<&MetadataMap>,
    mode: EmbeddingMode,
) -> Result<C::Response, SemanticEmbeddingError<C::Error>>
where
    R: ConfiguredTokenLimits + Sync,
    C: EmbeddingCalls,
{
    let request = service.prepare(prompt, metadata).await?;
    service.call(&request, mode).await
}
