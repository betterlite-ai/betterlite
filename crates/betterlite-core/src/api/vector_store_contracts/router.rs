use async_trait::async_trait;
use betterlite_config::types::ModelDeployment;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_provider::api::vector_store_contracts::config::{
    Params, VectorStoreEmbeddingExecutor,
};
use betterlite_provider::api::vector_store_contracts::error::{
    BaseLlmException, VectorStoreEmbeddingError,
};

use super::executor::DirectEmbeddingExecutor;

// the seam a router caller fills: what it resolves, what it serves and how it executes an embedding
#[async_trait]
pub trait VectorStoreRouter: Send + Sync {
    fn resolved_litellm_models(&self, model: &str, team_id: Option<&str>) -> Vec<String>;

    fn model_list(&self) -> Vec<ModelDeployment>;

    async fn embedding(
        &self,
        model: &str,
        input: &[String],
        configuration: Params,
    ) -> Result<EmbeddingResponse, BaseLlmException>;

    async fn aembedding(
        &self,
        model: &str,
        input: &[String],
        configuration: Params,
    ) -> Result<EmbeddingResponse, BaseLlmException>;
}

pub struct RouterEmbeddingExecutor<R> {
    pub router: R,
    pub metadata: Params,
    pub direct: DirectEmbeddingExecutor,
}

impl<R: VectorStoreRouter> RouterEmbeddingExecutor<R> {
    // configured metadata loses to the request metadata, and model/input never reach the embedding call
    fn embedding_kwargs(&self, configuration: &Params) -> Params {
        let mut kwargs: Params = configuration
            .iter()
            .filter(|(name, _)| !matches!(name.as_str(), "input" | "metadata" | "model"))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        let mut metadata: Params = match configuration.get("metadata") {
            Some(Json::Object(configured)) => configured
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            _ => Params::new(),
        };
        metadata.extend(
            self.metadata
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
        kwargs.insert(
            "metadata".to_owned(),
            Json::Object(metadata.into_iter().collect()),
        );
        kwargs
    }

    fn router_serves(&self, model: &str) -> bool {
        let team_id = self
            .metadata
            .get("user_api_key_team_id")
            .and_then(|value| match value {
                Json::Text(team) => Some(team.as_str()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    None
                }
            });
        if !self
            .router
            .resolved_litellm_models(model, team_id)
            .is_empty()
        {
            return true;
        }
        self.router
            .model_list()
            .iter()
            .any(|deployment| deployment.litellm_params.model.as_deref() == Some(model))
    }
}

#[async_trait]
impl<R: VectorStoreRouter> VectorStoreEmbeddingExecutor for RouterEmbeddingExecutor<R> {
    async fn embed(
        &self,
        model: &str,
        query: &str,
        configuration: &Params,
    ) -> Result<EmbeddingResponse, VectorStoreEmbeddingError> {
        let kwargs = self.embedding_kwargs(configuration);
        let input = [query.to_owned()];
        if self.router_serves(model) {
            return self
                .router
                .embedding(model, &input, kwargs)
                .await
                .map_err(|source| VectorStoreEmbeddingError::Router { source });
        }
        self.direct.embed(model, query, &kwargs).await
    }
    async fn aembed(
        &self,
        model: &str,
        query: &str,
        configuration: &Params,
    ) -> Result<EmbeddingResponse, VectorStoreEmbeddingError> {
        let kwargs = self.embedding_kwargs(configuration);
        let input = [query.to_owned()];
        if self.router_serves(model) {
            return self
                .router
                .aembedding(model, &input, kwargs)
                .await
                .map_err(|source| VectorStoreEmbeddingError::Router { source });
        }
        self.direct.aembed(model, query, &kwargs).await
    }
}
