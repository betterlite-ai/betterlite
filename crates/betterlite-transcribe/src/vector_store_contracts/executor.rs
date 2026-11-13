use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use betterlite_context::owned_parameter_names::is_litellm_owned_kwarg;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::metadata::identity::Mode;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::embedding::{
    EmbeddingError, EmbeddingRequest, EmbeddingResponse, send as send_embedding,
};
use betterlite_provider::api::vector_store_contracts::config::{
    EmbeddingEntry, Params, VectorStoreEmbeddingExecutor,
};
use betterlite_provider::api::vector_store_contracts::error::VectorStoreEmbeddingError;
use secrecy::SecretString;

// default_params of litellm.embedding and its named arguments, minus user, dimensions and encoding_format, which are sent
const CALL_ARGUMENTS: [&str; 15] = [
    "additional_drop_params",
    "aembedding",
    "api_base",
    "api_key",
    "api_type",
    "api_version",
    "base_url",
    "default_headers",
    "deployment_id",
    "drop_params",
    "extra_headers",
    "max_retries",
    "organization",
    "request_timeout",
    "timeout",
];
// litellm.embedding's own timeout argument, which a model no deployment serves waits on
const EMBEDDING_TIMEOUT_SECONDS: f64 = 600.0;

// LiteLLM's direct executor splats the configuration into the embedding call after model and input
pub struct DirectEmbeddingExecutor {
    pub executor: ChatExecutor,
    pub deployments: Vec<ChatDeployment>,
    pub defaults: OpenAiDefaults,
    pub catalog: Arc<ModelCatalog>,
}

#[async_trait]
impl VectorStoreEmbeddingExecutor for DirectEmbeddingExecutor {
    // LiteLLM's embed and aembed send the same request, so the entry point changes nothing here
    async fn embed(
        &self,
        _entry: EmbeddingEntry,
        model: &str,
        query: &str,
        configuration: &Params,
    ) -> Result<EmbeddingResponse, VectorStoreEmbeddingError> {
        let mut deployment = self.deployment(model, configuration)?;
        apply_call_options(&mut deployment, configuration)?;
        let request = embedding_request(query, configuration)?;
        send_embedding(&self.executor, &deployment, &request)
            .await
            .map(|outcome| outcome.response)
            .map_err(|source| VectorStoreEmbeddingError::Embedding { source })
    }
}

impl DirectEmbeddingExecutor {
    // a model no deployment serves is still called, from the configuration and the environment defaults
    pub(super) fn deployment(
        &self,
        model: &str,
        configuration: &Params,
    ) -> Result<ChatDeployment, VectorStoreEmbeddingError> {
        let matched = self.deployments.iter().find(|deployment| {
            deployment.litellm_model == model || deployment.model_group == model
        });
        if let Some(found) = matched {
            return Ok(found.clone());
        }
        ChatDeployment::resolve(
            &ModelDeployment {
                model_name: model.to_owned(),
                litellm_params: LiteLlmParams {
                    ssl_verify: None,
                    model: Some(self.served_model(model, configuration)?),
                    api_base: None,
                    api_key: None,
                    api_version: None,
                    timeout: Some(EMBEDDING_TIMEOUT_SECONDS),
                    stream_timeout: None,
                    max_retries: None,
                    rpm: None,
                    tpm: None,
                    weight: None,
                },
                model_info: ModelInfo::default(),
            },
            &self.defaults,
        )
        .map_err(|source| VectorStoreEmbeddingError::Deployment { source })
    }

    // get_llm_provider: a named provider prefixes the model, and a known OpenAI embedding model is openai's
    fn served_model(
        &self,
        model: &str,
        configuration: &Params,
    ) -> Result<String, VectorStoreEmbeddingError> {
        if let Some(provider) = text_option(configuration, "custom_llm_provider")? {
            if model.split('/').next() == Some(provider) {
                return Ok(model.to_owned());
            }
            return Ok(format!("{provider}/{model}"));
        }
        let known = self.catalog.metadata(model);
        if known
            .is_some_and(|entry| entry.provider == "openai" && entry.mode == Some(Mode::Embedding))
        {
            return Ok(format!("openai/{model}"));
        }
        Ok(model.to_owned())
    }
}

// a null option is an absent one, so the deployment keeps its own value
fn text_option<'configuration>(
    configuration: &'configuration Params,
    field: &'static str,
) -> Result<Option<&'configuration str>, VectorStoreEmbeddingError> {
    match configuration.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value)),
        Some(_) => Err(VectorStoreEmbeddingError::FieldType {
            field,
            expected: "string",
        }),
    }
}

fn apply_call_options(
    deployment: &mut ChatDeployment,
    configuration: &Params,
) -> Result<(), VectorStoreEmbeddingError> {
    if let Some(base) = text_option(configuration, "api_base")? {
        deployment
            .override_base(base)
            .map_err(|source| VectorStoreEmbeddingError::Embedding {
                source: EmbeddingError::Endpoint { source },
            })?;
    }
    if let Some(key) = text_option(configuration, "api_key")? {
        deployment.api_key = Some(SecretString::from(key.to_owned()));
    }
    match configuration.get("timeout") {
        None | Some(Json::Null) => {}
        Some(Json::Number(number)) => {
            // as_f64 only misses for arbitrary-precision numbers, and NaN is refused as a wait below
            let seconds = number.as_f64().unwrap_or(f64::NAN);
            deployment.timeout = Duration::try_from_secs_f64(seconds)
                .map_err(|source| VectorStoreEmbeddingError::Timeout { seconds, source })?;
            deployment.timeout_seconds = seconds;
        }
        Some(_) => {
            return Err(VectorStoreEmbeddingError::FieldType {
                field: "timeout",
                expected: "number",
            });
        }
    }
    Ok(())
}

// additional_drop_params names the optional parameters to leave out, and only a list counts
fn dropped_names(configuration: &Params) -> Result<Vec<&str>, VectorStoreEmbeddingError> {
    let wrong_type = VectorStoreEmbeddingError::FieldType {
        field: "additional_drop_params",
        expected: "list of strings",
    };
    let items = match configuration.get("additional_drop_params") {
        None | Some(Json::Null) => return Ok(Vec::new()),
        Some(Json::List(items)) => items,
        Some(_) => return Err(wrong_type),
    };
    let mut names = Vec::new();
    for item in items {
        match item {
            Json::Text(name) => names.push(name.as_str()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                return Err(wrong_type);
            }
        }
    }
    Ok(names)
}

fn merge_extra_body(
    body: &mut Params,
    configuration: &Params,
) -> Result<(), VectorStoreEmbeddingError> {
    match configuration.get("extra_body") {
        None | Some(Json::Null) => {}
        Some(Json::Object(extra)) => {
            body.extend(
                extra
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
        }
        Some(_) => {
            return Err(VectorStoreEmbeddingError::FieldType {
                field: "extra_body",
                expected: "object",
            });
        }
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
    let dropped = dropped_names(configuration)?;
    for (name, value) in configuration {
        match name.as_str() {
            "model" | "input" => {
                return Err(VectorStoreEmbeddingError::DuplicateArgument { name: name.clone() });
            }
            "extra_body" => {}
            // main.py writes encoding_format back after the drop, except the text none
            "encoding_format" if matches!(value, Json::Text(format) if format.trim().eq_ignore_ascii_case("none")) =>
                {}
            "dimensions" | "user" if dropped.contains(&name.as_str()) => {}
            // only openai is served, and LiteLLM keeps aws_*, vertex_* and hf_model_name for other providers
            option
                if CALL_ARGUMENTS.contains(&option)
                    || is_litellm_owned_kwarg(option)
                    || option.starts_with("aws_")
                    || option.starts_with("vertex_")
                    || option == "hf_model_name" => {}
            _ if matches!(value, Json::Null) => {}
            _ => {
                body.insert(name.clone(), value.clone());
            }
        }
    }
    merge_extra_body(&mut body, configuration)?;
    // B006's parser owns the typed request, including the passthrough map for provider parameters
    let bytes = serde_json::to_vec(&body).map_err(|source| VectorStoreEmbeddingError::Request {
        source: EmbeddingError::Encode { source },
    })?;
    EmbeddingRequest::parse(&bytes).map_err(|source| VectorStoreEmbeddingError::Request { source })
}
