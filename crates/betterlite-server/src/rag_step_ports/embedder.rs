use std::sync::Arc;

use async_trait::async_trait;
use betterlite_rag::s3_vectors_workflow::dimension::{EmbedError, RagEmbedder};

mod tests;

// the litellm.aembedding call over a list of chunk texts, the router's when the state holds one
pub struct ListEmbedder {
    pub router: Option<Arc<dyn RagEmbedder>>,
    pub native: Arc<dyn RagEmbedder>,
}

#[async_trait]
impl RagEmbedder for ListEmbedder {
    async fn embed(&self, model: &str, input: Vec<String>) -> Result<Vec<Vec<f64>>, EmbedError> {
        let backend: &dyn RagEmbedder = match &self.router {
            Some(router) => router.as_ref(),
            None => self.native.as_ref(),
        };
        let chunks = input.len();
        let vectors = backend.embed(model, input).await?;
        let returned = vectors.len();
        if returned != chunks {
            return Err(EmbedError::Failed {
                model: model.to_owned(),
                message: format!(
                    "the embedding response returned {returned} vectors for {chunks} chunks"
                ),
            });
        }
        Ok(vectors)
    }
}
