mod counter;
mod executor;
mod tests;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::vertex_embedding_dispatch::Settings as EmbeddingSettings;
use betterlite_config::types::Config;
use betterlite_core::operation::embedding::EmbeddingCall;
use betterlite_core::vertex_embedding_dispatch::{Pinned, VertexEmbeddingDispatchError, pin};
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::ModelCatalog;
use betterlite_openai::chat::deployment::{ChatDeployment, Provider as DeploymentProvider};
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_provider::vertex_embedding_binding::calls::VertexEmbeddingCalls;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::settings::Settings as CountSettings;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::HubDownloader;
use betterlite_tokenizer::engine::limits::{BlockingCounter, CountingService};
use betterlite_tokenizer::engine::selection::Selector;
use betterlite_vertex::credential_sources::sdk::{
    CredentialClock, GoogleAuth, HttpPolicy, SystemClock,
};
use betterlite_vertex::token_cache::CredentialCache;
use betterlite_vertex::token_refresh::CredentialRefresh;
use thiserror::Error;
use tokio::time::Instant;
use tokio_util::task::TaskTracker;

use counter::{InlineCounter, RuntimeTextCounter};
use executor::VertexCredentialExecutor;

// the anyio limiter LiteLLM counts tokens under, as the tokenizer's own tests size it
const TOKEN_COUNT_CONCURRENCY: usize = 4;

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
        counter: token_counter(config, &transport, &catalog)?,
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

// the chain the tokenizer's own tests build: a bundled vocabulary needs no download, and the hub is reached only for a custom tokenizer
fn token_counter(
    config: &Config,
    client: &Arc<ProxiedClient>,
    catalog: &Arc<ModelCatalog>,
) -> Result<Counter, VertexEmbeddingSetupError> {
    let settings = CountSettings::from_config(&config.sections.token_counting);
    let limits = settings
        .encode_limits()
        .map_err(|source| VertexEmbeddingSetupError::Limits { source })?;
    let facility: Arc<dyn BlockingCounter> = Arc::new(InlineCounter);
    let service = Arc::new(CountingService::new(
        TOKEN_COUNT_CONCURRENCY,
        facility,
        limits,
    ));
    let fetcher = Arc::new(TiktokenFetcher::new(
        Arc::clone(client),
        None,
        Arc::clone(&service),
    ));
    let selector = Arc::new(Selector::new(
        Arc::clone(&fetcher),
        hub_downloader(config, client),
        Arc::clone(catalog),
        Arc::clone(&service),
        false,
    ));
    Ok(Counter::new(
        selector,
        service,
        Arc::clone(catalog),
        settings,
    ))
}

fn hub_downloader(config: &Config, client: &Arc<ProxiedClient>) -> Arc<HubDownloader> {
    let hub = config.sections.huggingface_hub.clone();
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    let hub_clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    let resolver = Arc::new(HubTokenResolver::new(
        hub.clone(),
        config.sections.huggingface_oidc.clone(),
        Arc::clone(client),
        reader,
        hub_clock,
    ));
    Arc::new(HubDownloader::new(
        Arc::new(hub),
        resolver,
        Arc::new(Instant::now),
    ))
}
