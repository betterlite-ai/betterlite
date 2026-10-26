use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::Config;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
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
    Deployment {
        #[from]
        source: DeploymentError,
    },
    #[error("cannot build the outbound HTTP client: {source}")]
    Client {
        #[from]
        source: HttpError,
    },
}

// reads certificate files, so it runs before the async runtime starts
pub fn chat_executor(
    config: &Config,
    environment: &BTreeMap<String, String>,
    random: StdRng,
) -> Result<ChatExecutor, ChatSetupError> {
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
        organization: sdk.organization.clone().or_else(|| sdk.openai_organization.clone()),
    };
    let deployments = config
        .model_list
        .iter()
        .map(|deployment| ChatDeployment::resolve(deployment, &defaults))
        .collect::<Result<Vec<_>, _>>()?;
    let policy = Arc::new(ProxyPolicy::resolve(
        &config.litellm_settings.proxy_trust,
        environment,
    ));
    Ok(ChatExecutor {
        client: ProxiedClient::outbound(&policy)?,
        deployments,
        // the model cost map is not loaded yet, so no model counts as a catalog model
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
        random: Mutex::new(random),
    })
}
