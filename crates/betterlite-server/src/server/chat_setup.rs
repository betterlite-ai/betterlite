use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::mock_response::selection::MockPermission;
use betterlite_config::tls::{TlsPolicy, TlsPolicyError};
use betterlite_config::types::{Config, RoutingStrategy};
use betterlite_core::mock::chat::response as mock_chat;
use betterlite_core::mock::embedding::response as mock_embedding;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::{ClientChoice, ClientPool, PoolError};
use betterlite_openai::chat::MockSettings;
use betterlite_openai::chat::deployment::{ChatDeployment, DeploymentError, OpenAiDefaults};
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use rand::rngs::StdRng;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use thiserror::Error;

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
        "router_settings.routing_strategy {strategy:?} is not supported, only simple-shuffle is"
    )]
    RoutingStrategy { strategy: RoutingStrategy },
    #[error("cannot build the outbound HTTP client: {source}")]
    Client {
        #[from]
        source: PoolError,
    },
}

// reads certificate files, so it runs before the async runtime starts
pub fn chat_executor(
    config: &Config,
    environment: &BTreeMap<String, String>,
    random: StdRng,
) -> Result<ChatExecutor, ChatSetupError> {
    if let Some(strategy) = config.router_settings.routing_strategy
        && strategy != RoutingStrategy::SimpleShuffle
    {
        return Err(ChatSetupError::RoutingStrategy { strategy });
    }
    let defaults = openai_defaults(config);
    let deployments = config
        .model_list
        .iter()
        .map(|deployment| ChatDeployment::resolve(deployment, &defaults))
        .collect::<Result<Vec<_>, _>>()?;
    let client = outbound_client(config, environment, &deployments)?;
    let mock = mock_settings(config, &deployments);
    Ok(ChatExecutor {
        client,
        deployments,
        // the model cost map is not loaded yet, so no model counts as a catalog model
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
        random: Mutex::new(random),
        mock,
        mock_chat,
        mock_embedding,
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

fn mock_settings(config: &Config, deployments: &[ChatDeployment]) -> MockSettings {
    let mock_response = &config.sections.mock_response;
    let mut responses = BTreeMap::new();
    for (operator, deployment) in mock_response.deployments.iter().zip(deployments) {
        if let Some(response) = operator {
            responses.insert(deployment.model_id.clone(), response.clone());
        }
    }
    MockSettings {
        responses,
        usage: mock_response.usage,
        // auth metadata is not wired yet, so a client mock field is always stripped until B052 and B057
        permission: MockPermission::default(),
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

mod tests;
