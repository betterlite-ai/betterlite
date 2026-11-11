use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, TlsPolicyError};
use betterlite_config::types::{Config, ModelDeployment, RoutingStrategy};
use betterlite_core::mock::chat::response as mock_chat;
use betterlite_core::mock::embedding::response as mock_embedding;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::{ClientChoice, ClientPool, PoolError};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::chat::deployment::{
    ChatDeployment, DeploymentError, OpenAiDefaults, Provider as DeploymentProvider,
};
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_provider::aiohttp_openai_dispatch::{DEFAULT_AIOHTTP_BASE, complete_url};
use betterlite_provider::provider::Provider;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rand::rngs::StdRng;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use url::{ParseError, Url};

use mock::mock_settings;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ChatSetupError {
    #[error(transparent)]
    Environment {
        #[from]
        source: betterlite_env::EnvError,
    },
    #[error(transparent)]
    Deployment {
        #[from]
        source: DeploymentError,
    },
    #[error(transparent)]
    Tls {
        #[from]
        source: TlsPolicyError,
    },
    #[error(
        "model_list entry {model_name} has an api_base the aiohttp_openai provider cannot use: {source}"
    )]
    AiohttpBase {
        model_name: String,
        #[source]
        source: ParseError,
    },
    #[error(
        "router_settings.routing_strategy {strategy:?} is not supported, only simple-shuffle is"
    )]
    RoutingStrategy { strategy: RoutingStrategy },
    #[error("cannot build the outbound HTTP client: {source}")]
    Client {
        #[from]
        source: PoolError,
    },
    #[error(transparent)]
    Redactor {
        #[from]
        source: RedactionError,
    },
}

// reads certificate files, so it runs before the async runtime starts
pub fn chat_executor(
    config: &Config,
    environment: &BTreeMap<String, String>,
    random: StdRng,
) -> Result<ChatExecutor, ChatSetupError> {
    reject_other_strategies(config)?;
    let defaults = openai_defaults(config);
    // the bundled model cost map lets a deployment without a provider prefix resolve like LiteLLM's
    let catalog = BundledCatalog::default().get();
    let known_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let resolution = resolution_context(
        config,
        &catalog,
        environment,
        &known_names,
        &custom_providers,
    );
    let mut deployments = chat_deployments(config, &defaults, &resolution)?;
    let mock = mock_settings(config, &mut deployments);
    Ok(ChatExecutor {
        client: outbound_client(config, environment, &deployments)?,
        deployments,
        // the model cost map is not loaded yet, so no model counts as a catalog model
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
        random: Mutex::new(random),
        clock: unix_seconds,
        next_id: Uuid::new_v4,
        mock,
        mock_chat,
        mock_embedding,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?,
    })
}

fn openai_defaults(config: &Config) -> OpenAiDefaults {
    let sdk = &config.sections.resource_sdk_settings;
    OpenAiDefaults {
        request_timeout: config.litellm_settings.request_timeout,
        api_bases: [&sdk.api_base, &sdk.openai_base_url, &sdk.openai_api_base]
            .into_iter()
            .flatten()
            .cloned()
            .collect(),
        api_keys: [&sdk.api_key, &sdk.openai_key, &sdk.openai_api_key]
            .into_iter()
            .flatten()
            .cloned()
            .collect(),
        organization: sdk
            .organization
            .clone()
            .or_else(|| sdk.openai_organization.clone()),
    }
}

fn resolution_context<'context>(
    config: &Config,
    catalog: &'context ModelCatalog,
    environment: &'context BTreeMap<String, String>,
    known_names: &'context BTreeSet<String>,
    custom_providers: &'context BTreeSet<String>,
) -> ResolutionContext<'context> {
    let sdk = &config.sections.resource_sdk_settings;
    ResolutionContext {
        catalog,
        known_model_names: known_names,
        custom_providers,
        environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials {
            api_key: sdk.api_key.clone(),
            api_base: sdk.api_base.clone(),
            xai_key: None,
            inception_key: None,
            lemonade_key: None,
        },
    }
}

// get_llm_provider runs once per deployment, with the custom_llm_provider its config entry declares
fn chat_deployments(
    config: &Config,
    defaults: &OpenAiDefaults,
    resolution: &ResolutionContext<'_>,
) -> Result<Vec<ChatDeployment>, ChatSetupError> {
    let identity = &config.sections.provider_resolution;
    let mut resolved = Vec::new();
    for (index, deployment) in config.model_list.iter().enumerate() {
        let custom_llm_provider = identity.deployments.get(index).and_then(Option::as_deref);
        let mut chat =
            ChatDeployment::resolve_with(deployment, defaults, custom_llm_provider, resolution)?;
        prefix_provider(&mut chat, custom_llm_provider);
        if matches!(
            Provider::from_deployment(&chat),
            Ok(Provider::AiohttpOpenAi(_))
        ) {
            apply_aiohttp_defaults(&mut chat, deployment)?;
        }
        resolved.push(chat);
    }
    Ok(resolved)
}

// the resolver keeps only the model, so the provider LiteLLM took from custom_llm_provider goes back on as a prefix
fn prefix_provider(resolved: &mut ChatDeployment, custom_llm_provider: Option<&str>) {
    let (Some(slug), DeploymentProvider::Unsupported { model }) =
        (custom_llm_provider, &mut resolved.provider)
    else {
        return;
    };
    if !model.starts_with(&format!("{slug}/")) {
        *model = format!("{slug}/{model}");
    }
}

// aiohttp_openai reads only its own deployment values: the OpenAI environment defaults never apply to it
fn apply_aiohttp_defaults(
    resolved: &mut ChatDeployment,
    deployment: &ModelDeployment,
) -> Result<(), ChatSetupError> {
    let base = deployment
        .litellm_params
        .api_base
        .as_deref()
        .unwrap_or(DEFAULT_AIOHTTP_BASE);
    resolved.endpoint =
        Url::parse(&complete_url(base)).map_err(|source| ChatSetupError::AiohttpBase {
            model_name: resolved.model_group.clone(),
            source,
        })?;
    base.clone_into(&mut resolved.api_base);
    resolved
        .api_key
        .clone_from(&deployment.litellm_params.api_key);
    resolved.organization = None;
    Ok(())
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn reject_other_strategies(config: &Config) -> Result<(), ChatSetupError> {
    match config.router_settings.routing_strategy {
        Some(strategy) if strategy != RoutingStrategy::SimpleShuffle => {
            Err(ChatSetupError::RoutingStrategy { strategy })
        }
        Some(_) | None => Ok(()),
    }
}

fn outbound_client(
    config: &Config,
    environment: &BTreeMap<String, String>,
    deployments: &[ChatDeployment],
) -> Result<ProxiedClient, ChatSetupError> {
    let policy = Arc::new(ProxyPolicy::resolve(
        &config.litellm_settings.proxy_trust,
        environment,
    )?);
    // one entry for the default policy and one more for each deployment whose ssl_verify differs
    let pool = ClientPool::new(
        deployments.len().saturating_add(1),
        TransportLimits::DEFAULT,
    );
    let default_tls = TlsPolicy::resolve(config, environment, None)?;
    let network_mock = config.sections.openai_mock_transport.network_mock;
    let mut client = match pool.select(None, network_mock, &default_tls, &policy)? {
        ClientChoice::Supplied(client) | ClientChoice::Pooled(client) => client,
        ClientChoice::Mock => {
            let mut mocked = pool.client(&default_tls, &policy)?;
            mocked.network_mock = true;
            return Ok(mocked);
        }
    };
    let mut by_deployment = BTreeMap::new();
    for (resolved, entry) in deployments.iter().zip(&config.model_list) {
        let tls = TlsPolicy::resolve(config, environment, entry.litellm_params.ssl_verify)?;
        by_deployment.insert(resolved.model_id.clone(), pool.client(&tls, &policy)?);
    }
    client.deployments = Arc::new(by_deployment);
    Ok(client)
}

mod mock;
pub(crate) mod provider_call;
mod tests;
