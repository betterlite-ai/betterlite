mod counter;
mod executor;
mod tests;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::vertex_embedding_dispatch::Settings as EmbeddingSettings;
use betterlite_config::types::Config;
use betterlite_core::operation::embedding::EmbeddingCall;
use betterlite_core::vertex_embedding_dispatch::{Pinned, VertexEmbeddingDispatchError, pin};
use betterlite_openai::chat::deployment::{ChatDeployment, Provider as DeploymentProvider};
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_provider::vertex_embedding_binding::calls::VertexEmbeddingCalls;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::runtime::runtime_counter;
use betterlite_vertex::credential_sources::sdk::{
    CredentialClock, GoogleAuth, HttpPolicy, SystemClock,
};
use betterlite_vertex::token_cache::CredentialCache;
use betterlite_vertex::token_refresh::CredentialRefresh;
use thiserror::Error;
use tokio_util::task::TaskTracker;

use counter::RuntimeTextCounter;
use executor::VertexCredentialExecutor;

#[derive(Debug, Error)]
pub enum VertexEmbeddingSetupError {
    #[error("model_list entry {model_name} pins a Vertex option that cannot be read: {source}")]
    Pinned {
        model_name: String,
        #[source]
        source: VertexEmbeddingDispatchError,
    },
    #[error("the token counter limits are invalid: {source}")]
    Limits {
        #[source]
        source: CountError,
    },
}

// None when no deployment is a vertex_ai one, so a config without Vertex builds no credential or tokenizer state
pub fn vertex_embedding_calls(
    config: &Config,
    chat: &ChatExecutor,
    tasks: &TaskTracker,
) -> Result<Option<Arc<dyn EmbeddingCall>>, VertexEmbeddingSetupError> {
    let embedding = &config.sections.vertex_embedding_dispatch;
    let pinned = pinned_deployments(&chat.deployments, embedding)?;
    if pinned.is_empty() {
        return Ok(None);
    }
    let catalog = Arc::clone(&chat.bundled_catalog);
    let transport = Arc::new(chat.client.clone());
    let clock: Arc<dyn CredentialClock> = Arc::new(SystemClock);
    let executor = Arc::new(VertexCredentialExecutor {
        environment: config.sections.vertex_credential_sources.adc.clone(),
        google: GoogleAuth {
            client: chat.client.clone(),
            policy: HttpPolicy::default(),
        },
        clock: Arc::clone(&clock),
        handles: Mutex::new(BTreeMap::new()),
    });
    let cache = Arc::new(CredentialCache::new(clock, executor));
    let refresh = Arc::new(CredentialRefresh::new(Arc::clone(&cache), tasks.clone()));
    let counter = RuntimeTextCounter {
        counter: runtime_counter(config, &transport, &catalog)
            .map_err(|source| VertexEmbeddingSetupError::Limits { source })?,
    };
    Ok(Some(Arc::new(VertexEmbeddingCalls {
        catalog,
        cache,
        refresh,
        counter: Arc::new(counter),
        transport,
        vertex: config.sections.vertex_credential_sources.clone(),
        embedding: embedding.clone(),
        pinned,
    })))
}

fn pinned_deployments(
    deployments: &[ChatDeployment],
    embedding: &EmbeddingSettings,
) -> Result<BTreeMap<String, Pinned>, VertexEmbeddingSetupError> {
    let mut pinned = BTreeMap::new();
    for (deployment, params) in deployments.iter().zip(&embedding.deployments) {
        let DeploymentProvider::Unsupported { model } = &deployment.provider else {
            continue;
        };
        if !model.starts_with("vertex_ai/") {
            continue;
        }
        let pins = pin(&params.0).map_err(|source| VertexEmbeddingSetupError::Pinned {
            model_name: deployment.model_group.clone(),
            source,
        })?;
        pinned.insert(deployment.model_id.clone(), pins);
    }
    Ok(pinned)
}
