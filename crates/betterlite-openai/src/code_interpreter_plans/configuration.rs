use std::sync::Arc;

use betterlite_models::catalog::ModelCatalog;
use serde::Deserialize;

use super::initialization::InterceptionLogger;

// CodeInterpreterInterceptionConfig from LiteLLM's types: every key optional, enabled defaulting to true
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InterceptionConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub enabled_providers: Option<Vec<String>>,
    pub sandbox_tool_name: Option<String>,
}

const fn default_enabled() -> bool {
    true
}

impl Default for InterceptionConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            enabled_providers: None,
            sandbox_tool_name: None,
        }
    }
}

// The two places LiteLLM reads the interceptor's settings from: the litellm_settings entry wins over the callback-specific one
#[derive(Debug, Clone, Default)]
pub struct ProxyParameters {
    pub interception_params: Option<InterceptionConfig>,
    pub callback_specific_params: Option<InterceptionConfig>,
}

#[must_use]
pub fn from_config_yaml<C>(
    config: &InterceptionConfig,
    catalog: Arc<ModelCatalog>,
) -> InterceptionLogger<C> {
    InterceptionLogger::new(
        config.enabled,
        config.enabled_providers.clone(),
        config.sandbox_tool_name.clone(),
        None,
        catalog,
    )
}

#[must_use]
pub fn initialize_from_proxy_config<C>(
    params: &ProxyParameters,
    catalog: Arc<ModelCatalog>,
) -> InterceptionLogger<C> {
    let Some(config) = params
        .interception_params
        .as_ref()
        .or(params.callback_specific_params.as_ref())
    else {
        return from_config_yaml(&InterceptionConfig::default(), catalog);
    };
    from_config_yaml(config, catalog)
}

include!("configuration/tests/cases.rs");
