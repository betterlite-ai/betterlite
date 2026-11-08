use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::vector_query_embedding::query::embed_query;
use betterlite_core::api::vector_store_contracts::executor::DirectEmbeddingExecutor;
use betterlite_core::mock::{chat::response as mock_chat, embedding::response as mock_embedding};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::MockSettings;
use betterlite_openai::chat::deployment::OpenAiDefaults;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_provider::api::vector_query_embedding::error::QueryEmbeddingError;
use betterlite_provider::api::vector_store_contracts::config::{EmbeddingEntry, Params};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use serde_json::Number;
use uuid::Uuid;

use super::VectorQueryEmbeddingCall;
use super::error::SdkHttpError;
use super::native::openai_defaults;
use super::result::{Failure, QueryVectors, SdkResult};

// LiteLLM's embed_query runs first, and its aembed_query only runs when the first call returned
pub(crate) async fn send(
    call: &VectorQueryEmbeddingCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let client = outbound_client(env)?;
    let defaults = environment_defaults(env)?;
    let litellm_params = with_api_base(&call.litellm_params, api_base);
    let sync = match embed_query(
        EmbeddingEntry::Embedding,
        &call.query_text,
        &litellm_params,
        None,
        direct_executor(&client, &defaults),
    )
    .await
    {
        Ok(vector) => vector,
        Err(error) => return Ok(failure(&error)),
    };
    let asynchronous = match embed_query(
        EmbeddingEntry::Aembedding,
        &call.query_text,
        &litellm_params,
        None,
        direct_executor(&client, &defaults),
    )
    .await
    {
        Ok(vector) => vector,
        Err(error) => return Ok(failure(&error)),
    };
    let numbers = |vector: Vec<f64>| -> Vec<Number> {
        vector.into_iter().filter_map(Number::from_f64).collect()
    };
    Ok(SdkResult::Vectors {
        value: QueryVectors {
            sync: numbers(sync),
            asynchronous: numbers(asynchronous),
        },
    })
}

fn outbound_client(env: &BTreeMap<String, String>) -> Result<ProxiedClient, SdkHttpError> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    ProxiedClient::outbound(&policy).map_err(|error| SdkHttpError::Client(Box::new(error)))
}

// the harness owns the mock's base, so it takes the place of the configured api_base
fn with_api_base(litellm_params: &Params, api_base: &str) -> Params {
    let mut params = litellm_params.clone();
    if let Some(Json::Object(configuration)) = params.get_mut("litellm_embedding_config") {
        configuration.insert("api_base".to_owned(), Json::Text(api_base.to_owned()));
    }
    params
}

fn environment_defaults(env: &BTreeMap<String, String>) -> Result<OpenAiDefaults, SdkHttpError> {
    let loader = ConfigLoader {
        lookup: env.clone(),
    };
    let config = loader
        .load("model_list: []")
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    Ok(openai_defaults(&config))
}

fn direct_executor(client: &ProxiedClient, defaults: &OpenAiDefaults) -> DirectEmbeddingExecutor {
    DirectEmbeddingExecutor {
        executor: ChatExecutor {
            client: client.clone(),
            deployments: Vec::new(),
            catalog: CatalogCapabilities {
                chat_models: BTreeSet::new(),
                text_models: BTreeSet::new(),
            },
            random: Mutex::new(StdRng::seed_from_u64(0)),
            clock: || 0,
            next_id: Uuid::new_v4,
            mock: MockSettings::default(),
            mock_chat,
            mock_embedding,
        },
        deployments: Vec::new(),
        defaults: defaults.clone(),
        catalog: BundledCatalog::default().get(),
    }
}

// the class each LiteLLM error projects to; a plain exception carries no status, type, param or code
fn failure(error: &QueryEmbeddingError) -> SdkResult {
    let class = match error {
        QueryEmbeddingError::MissingModel | QueryEmbeddingError::ConfigKey => "ValueError",
        QueryEmbeddingError::NoFirstRecord => "IndexError",
        QueryEmbeddingError::NotAVector | QueryEmbeddingError::Answer { .. } => "ValidationError",
        QueryEmbeddingError::Execution { .. } => "Exception",
    };
    SdkResult::Failure(Failure {
        class: class.to_owned(),
        status: 0,
        kind: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
