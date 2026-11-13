use super::EmbeddedReply as _;
use super::error::{EmbeddingReplyError, SemanticEmbeddingError};
use super::{
    ConfiguredTokenLimits, EmbeddingCalls, EmbeddingMode, MetadataMap, SemanticEmbeddingService,
};

pub async fn redis_embedding<R, C>(
    service: &SemanticEmbeddingService<R, C>,
    prompt: &str,
    metadata: Option<&MetadataMap>,
    mode: EmbeddingMode,
) -> Result<Vec<f64>, SemanticEmbeddingError<C::Error>>
where
    R: ConfiguredTokenLimits + Sync,
    C: EmbeddingCalls,
{
    let request = service.prepare(prompt, metadata).await?;
    let reply = service.call(&request, mode).await;
    let vector = reply.and_then(|response| {
        response
            .first_embedding()
            .ok_or(EmbeddingReplyError::NoVector)
            .map_err(SemanticEmbeddingError::Reply)
    });
    match mode {
        EmbeddingMode::Sync => vector,
        // LiteLLM's async redis path wraps the awaited call and the vector read, not the preparation
        EmbeddingMode::Async => vector.map_err(|source| SemanticEmbeddingError::Failed {
            source: Box::new(source),
        }),
    }
}
