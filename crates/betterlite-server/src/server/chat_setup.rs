use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::mock_response::MockResponse;
use betterlite_config::sections::mock_response::selection::MockPermission;
use betterlite_config::tls::{TlsPolicy, TlsPolicyError};
use betterlite_config::types::{Config, ModelDeployment, RoutingStrategy};
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
use secrecy::ExposeSecret;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
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
    reject_other_strategies(config)?;
    let sdk = &config.sections.resource_sdk_settings;
    let defaults = OpenAiDefaults {
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
    };
    let mut deployments = config
        .model_list
        .iter()
        .map(|deployment| ChatDeployment::resolve(deployment, &defaults))
        .collect::<Result<Vec<_>, _>>()?;
    let mock = mock_settings(config, &mut deployments);
    let client = outbound_client(config, environment, &deployments)?;
    Ok(ChatExecutor {
        client,
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
    })
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

// LiteLLM's generate_model_id hashes the model group and each litellm_params key and value, mock_response included
fn mock_model_id(entry: &ModelDeployment, mock: &MockResponse) -> Option<String> {
    if entry.model_info.id.is_some() {
        return None;
    }
    let params = &entry.litellm_params;
    let api_key = params.api_key.as_ref().map(ExposeSecret::expose_secret);
    let fields: [(&str, Option<String>); 10] = [
        ("model", params.model.clone()),
        ("api_base", params.api_base.clone()),
        ("api_key", api_key.map(str::to_owned)),
        ("api_version", params.api_version.clone()),
        ("timeout", params.timeout.map(|value| value.to_string())),
        (
            "stream_timeout",
            params.stream_timeout.map(|value| value.to_string()),
        ),
        (
            "max_retries",
            params.max_retries.map(|value| value.to_string()),
        ),
        ("rpm", params.rpm.map(|value| value.to_string())),
        ("tpm", params.tpm.map(|value| value.to_string())),
        ("weight", params.weight.map(|value| value.to_string())),
    ];
    let mut hasher = Sha256::new();
    hasher.update(entry.model_name.as_bytes());
    for (name, value) in fields {
        if let Some(text) = value {
            hasher.update(name.as_bytes());
            hasher.update(text.as_bytes());
        }
    }
    hasher.update(b"mock_response");
    hasher.update(mock_id_value(mock).as_bytes());
    Some(hex::encode(hasher.finalize()))
}

// a string is hashed as given and a number vector as Python prints it; an object keeps betterlite's JSON text (deviation)
fn mock_id_value(mock: &MockResponse) -> String {
    match mock {
        MockResponse::Text(text) => text.clone(),
        MockResponse::Numbers(values) => {
            let parts: Vec<String> = values.iter().map(ToString::to_string).collect();
            format!("[{}]", parts.join(", "))
        }
        MockResponse::Object(_) => serde_json::to_string(mock).unwrap_or_default(),
    }
}

// LiteLLM's model id hashes litellm_params, mock_response included, so deployments that differ only in it stay distinct
fn mock_settings(config: &Config, deployments: &mut [ChatDeployment]) -> MockSettings {
    let section = &config.sections.mock_response;
    let mut responses = BTreeMap::new();
    for (index, deployment) in deployments.iter_mut().enumerate() {
        let Some(response) = section.deployments.get(index).and_then(Option::as_ref) else {
            continue;
        };
        let entry = config.model_list.get(index);
        if let Some(id) = entry.and_then(|listed| mock_model_id(listed, response)) {
            deployment.model_id = id;
        }
        responses.insert(deployment.model_id.clone(), response.clone());
    }
    MockSettings {
        responses,
        usage: section.usage,
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
