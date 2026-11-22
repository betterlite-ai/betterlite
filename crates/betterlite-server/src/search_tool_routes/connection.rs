use std::time::Duration;

use betterlite_core::search::{SearchOptions, SearchRequest};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::state::{ConnectionFailure, ConnectionSearch, ProviderCatalog};

const TEST_QUERY: &str = "test";
const TEST_MAX_RESULTS: i64 = 1;
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

// SearchProviders in LiteLLM's enum order, which is the order available_providers lists them in
pub(super) const SEARCH_PROVIDERS: [&str; 20] = [
    "perplexity",
    "tavily",
    "parallel_ai",
    "exa_ai",
    "brave",
    "google_pse",
    "dataforseo",
    "firecrawl",
    "fastcrw",
    "searxng",
    "linkup",
    "duckduckgo",
    "searchapi",
    "serper",
    "you_com",
    "apiserpent",
    "tinyfish",
    "agentcore",
    "nimble",
    "bing_grounding",
];

#[derive(Deserialize)]
pub struct ConnectionBody {
    litellm_params: ConnectionParams,
}

// only these three keys are read; the rest of litellm_params is ignored, as LiteLLM ignores it
#[derive(Deserialize)]
struct ConnectionParams {
    #[serde(default)]
    search_provider: Option<String>,
    #[serde(default)]
    api_key: Option<SecretString>,
    #[serde(default)]
    api_base: Option<String>,
}

#[derive(Debug, Error)]
enum ConnectionError {
    // LiteLLM raises this HTTPException inside its own try block, so it becomes the error answer
    #[error("400: search_provider is required in litellm_params")]
    MissingProvider,
    #[error("{0}")]
    Search(#[source] ConnectionFailure),
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub(super) enum ConnectionAnswer {
    Success {
        message: String,
        test_query: &'static str,
        results_count: usize,
    },
    Error {
        message: String,
        error_type: &'static str,
    },
}

#[derive(Serialize)]
pub(super) struct ProvidersBody {
    providers: Vec<AvailableProvider>,
}

#[derive(Serialize)]
struct AvailableProvider {
    provider_name: &'static str,
    ui_friendly_name: String,
}

// every failure is a 200 answer with status error; only a body that fails validation gets an error status
pub(super) async fn test_connection(
    search: &dyn ConnectionSearch,
    body: ConnectionBody,
) -> ConnectionAnswer {
    match connect(search, body.litellm_params).await {
        Ok(answer) => answer,
        Err(error) => {
            tracing::error!("Failed to connect to search provider: {error}");
            let error_type = match &error {
                ConnectionError::MissingProvider => "HTTPException",
                ConnectionError::Search(failure) => failure.error_type,
            };
            ConnectionAnswer::Error {
                message: error.to_string(),
                error_type,
            }
        }
    }
}

async fn connect(
    search: &dyn ConnectionSearch,
    params: ConnectionParams,
) -> Result<ConnectionAnswer, ConnectionError> {
    let provider = match params.search_provider {
        Some(provider) if !provider.is_empty() => provider,
        Some(_) | None => return Err(ConnectionError::MissingProvider),
    };
    tracing::debug!("Testing connection to search provider: {provider}");
    let request = SearchRequest {
        query: Json::Text(TEST_QUERY.to_owned()),
        search_provider: provider.clone(),
        options: SearchOptions {
            max_results: Some(TEST_MAX_RESULTS),
            search_domain_filter: None,
            max_tokens_per_page: None,
            country: None,
        },
        api_key: params.api_key,
        api_base: params.api_base,
        timeout: Some(TEST_TIMEOUT),
        extra_headers: Headers::new(),
        kwargs: IndexMap::new(),
    };
    let response = search
        .search(request)
        .await
        .map_err(ConnectionError::Search)?;
    tracing::debug!("Successfully tested connection to {provider} search provider");
    Ok(ConnectionAnswer::Success {
        message: format!("Successfully connected to {provider} search provider"),
        test_query: TEST_QUERY,
        results_count: response.results.len(),
    })
}

// a provider without a config is left out quietly, and one whose lookup fails with a debug line
pub(super) fn available_providers(catalog: &dyn ProviderCatalog) -> ProvidersBody {
    let mut providers = Vec::with_capacity(SEARCH_PROVIDERS.len());
    for provider in SEARCH_PROVIDERS {
        match catalog.ui_friendly_name(provider) {
            Ok(Some(ui_friendly_name)) => providers.push(AvailableProvider {
                provider_name: provider,
                ui_friendly_name,
            }),
            Ok(None) => {}
            Err(error) => {
                tracing::debug!("Could not get config for search provider {provider}: {error}");
            }
        }
    }
    ProvidersBody { providers }
}
