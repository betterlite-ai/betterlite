use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::embedding::{
    EmbeddingError, EmbeddingRequest, EmbeddingResponse, send as send_embedding,
};
use betterlite_provider::api::vector_store_contracts::config::{
    Params, VectorStoreEmbeddingExecutor,
};
use betterlite_provider::api::vector_store_contracts::error::VectorStoreEmbeddingError;
use secrecy::SecretString;

// LiteLLM's direct executor splats the configuration into the embedding call after model and input
pub struct DirectEmbeddingExecutor {
    pub executor: ChatExecutor,
    pub deployments: Vec<ChatDeployment>,
}

#[async_trait]
impl VectorStoreEmbeddingExecutor for DirectEmbeddingExecutor {
    async fn embed(
        &self,
        model: &str,
        query: &str,
        configuration: &Params,
    ) -> Result<EmbeddingResponse, VectorStoreEmbeddingError> {
        self.send(model, query, configuration).await
    }
}

impl DirectEmbeddingExecutor {
    async fn send(
        &self,
        model: &str,
        query: &str,
        configuration: &Params,
    ) -> Result<EmbeddingResponse, VectorStoreEmbeddingError> {
        let deployment = self.deployment(model, configuration)?;
        let request = embedding_request(query, configuration)?;
        send_embedding(&self.executor, &deployment, &request)
            .await
            .map(|outcome| outcome.response)
            .map_err(|source| VectorStoreEmbeddingError::Embedding { source })
    }

    fn deployment(
        &self,
        model: &str,
        configuration: &Params,
    ) -> Result<ChatDeployment, VectorStoreEmbeddingError> {
        let found = self
            .deployments
            .iter()
            .find(|deployment| deployment.litellm_model == model || deployment.model_group == model)
            .ok_or_else(|| VectorStoreEmbeddingError::UnknownModel {
                model: model.to_owned(),
            })?;
        let mut deployment = found.clone();
        apply_call_options(&mut deployment, configuration)?;
        Ok(deployment)
    }
}

// the parameters litellm.embedding reads itself, which never reach the provider body
fn apply_call_options(
    deployment: &mut ChatDeployment,
    configuration: &Params,
) -> Result<(), VectorStoreEmbeddingError> {
    if let Some(value) = configuration.get("api_base") {
        let Json::Text(base) = value else {
            return Err(VectorStoreEmbeddingError::FieldType {
                field: "api_base",
                expected: "string",
            });
        };
        base.clone_into(&mut deployment.api_base);
    }
    if let Some(value) = configuration.get("api_key") {
        let Json::Text(key) = value else {
            return Err(VectorStoreEmbeddingError::FieldType {
                field: "api_key",
                expected: "string",
            });
        };
        deployment.api_key = Some(SecretString::from(key.clone()));
    }
    if let Some(value) = configuration.get("timeout") {
        let Json::Number(number) = value else {
            return Err(VectorStoreEmbeddingError::FieldType {
                field: "timeout",
                expected: "number",
            });
        };
        let seconds = number
            .as_f64()
            .ok_or(VectorStoreEmbeddingError::FieldValue {
                field: "timeout",
                expected: "number",
            })?;
        let wait = Duration::try_from_secs_f64(seconds)
            .map_err(|source| VectorStoreEmbeddingError::Timeout { seconds, source })?;
        deployment.timeout = wait;
        deployment.timeout_seconds = seconds;
    }
    Ok(())
}

fn embedding_request(
    query: &str,
    configuration: &Params,
) -> Result<EmbeddingRequest, VectorStoreEmbeddingError> {
    let mut body = Params::new();
    body.insert(
        "input".to_owned(),
        Json::List(vec![Json::Text(query.to_owned())]),
    );
    for (name, value) in configuration {
        match name.as_str() {
            "model" | "input" => {
                return Err(VectorStoreEmbeddingError::DuplicateArgument { name: name.clone() });
            }
            "api_base" | "api_key" | "metadata" | "timeout" => {}
            "dimensions" | "encoding_format" | "user" => {
                body.insert(name.clone(), value.clone());
            }
            _ => {
                return Err(VectorStoreEmbeddingError::UnsupportedParameter { name: name.clone() });
            }
        }
    }
    // B006's parser owns the typed request, including the passthrough map for provider parameters
    let bytes = serde_json::to_vec(&body).map_err(|source| VectorStoreEmbeddingError::Request {
        source: EmbeddingError::Encode { source },
    })?;
    EmbeddingRequest::parse(&bytes).map_err(|source| VectorStoreEmbeddingError::Request { source })
}
