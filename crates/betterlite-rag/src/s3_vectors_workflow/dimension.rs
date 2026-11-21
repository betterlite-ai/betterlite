use async_trait::async_trait;
use tracing::warn;

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("the embedding request for '{model}' failed: {message}")]
    Failed { model: String, message: String },
}

#[async_trait]
pub trait RagEmbedder: Send + Sync {
    async fn embed(&self, model: &str, input: Vec<String>) -> Result<Vec<Vec<f64>>, EmbedError>;
}

#[must_use]
pub async fn detect_dimension<Embedder>(
    embedder: &Embedder,
    embedding_model: Option<&str>,
    default_dimension: i64,
) -> i64
where
    Embedder: RagEmbedder,
{
    let Some(model) = embedding_model else {
        return default_dimension;
    };
    match embedder.embed(model, vec!["test".to_owned()]).await {
        Ok(vectors) => vectors.first().map_or(default_dimension, |vector| {
            i64::try_from(vector.len()).unwrap_or(default_dimension)
        }),
        Err(error) => {
            warn!(
                "Could not auto-detect dimension from embedding model: {error}. Using default dimension of {default_dimension}."
            );
            default_dimension
        }
    }
}
